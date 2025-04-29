use application::{ApplicationError, FacetCounts, Index, SearchResult, SortBy, SortOrder};
use async_trait::async_trait;
use dashmap::DashMap;
use domain::{CollectionSchema, Document, DocumentId, FieldType}; // Import Schema type
use serde_json::{Number, Value};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::{collections::HashMap, sync::Arc};
use tracing::{debug, info, instrument, trace, warn};

/// Represents the data stored for a single collection within the in-memory index.
#[derive(Debug, Default)]
struct CollectionIndexData {
    // Store documents for this specific collection
    documents: DashMap<DocumentId, Arc<Document>>,
    // Store the schema for reference (e.g., to know which fields are indexed)
    schema: Arc<CollectionSchema>,
}

/// In-memory index implementation supporting multiple collections.
/// WARNING: Still uses naive substring search and is NOT efficient.
#[derive(Debug, Clone, Default)]
pub struct InMemoryIndex {
    // Collection Name -> Collection Index Data
    collections: Arc<DashMap<String, Arc<CollectionIndexData>>>,
}

impl InMemoryIndex {
    pub fn new() -> Self {
        Self {
            collections: Arc::new(DashMap::new()),
        }
    }
}

#[async_trait]
impl Index for InMemoryIndex {
    #[instrument(skip(self, schema))]
    async fn ensure_collection_exists(
        &self,
        schema: &CollectionSchema,
    ) -> Result<(), ApplicationError> {
        let name = schema.name.clone();
        debug!(collection = %name, "Ensuring collection exists in in-memory index");
        if !self.collections.contains_key(&name) {
            info!(collection = %name, "Creating new index entry for collection");
            let collection_data = Arc::new(CollectionIndexData {
                documents: DashMap::new(),
                // Clone the schema into an Arc for the collection data
                schema: Arc::new(schema.clone()),
            });
            self.collections.insert(name, collection_data);
        } else {
            // Optional: Check if schema matches existing one and update/error if necessary?
            // For now, assume schema doesn't change after creation via this call.
            debug!(collection = %name, "Collection already exists in index");
        }
        Ok(())
    }

    #[instrument(skip(self, document))]
    async fn index_document(
        &self,
        collection_name: &str,
        document: &Document,
    ) -> Result<(), ApplicationError> {
        debug!(collection = %collection_name, doc_id = %document.id().as_str(), "Indexing document in-memory");
        match self.collections.get(collection_name) {
            Some(collection_data) => {
                collection_data
                    .documents
                    .insert(document.id().clone(), Arc::new(document.clone()));
                Ok(())
            }
            None => {
                // This should ideally not happen if ensure_collection_exists was called
                warn!(collection = %collection_name, "Attempted to index into non-existent collection index");
                Err(ApplicationError::CollectionNotFound(
                    collection_name.to_string(),
                ))
            }
        }
    }

    #[instrument(skip(self))]
    async fn delete_document(
        &self,
        collection_name: &str,
        id: &DocumentId,
    ) -> Result<(), ApplicationError> {
        debug!(collection = %collection_name, doc_id = %id.as_str(), "Removing document from in-memory index");
        if let Some(collection_data) = self.collections.get(collection_name) {
            collection_data.documents.remove(id);
            // It's okay if the document wasn't present
            Ok(())
        } else {
            warn!(collection = %collection_name, "Attempted to delete from non-existent collection index");
            // Still return Ok? Or CollectionNotFound? Let's be lenient for deletion.
            Ok(()) // If collection doesn't exist, document effectively doesn't either.
        }
    }

    #[instrument(skip(self))]
    async fn delete_collection(&self, collection_name: &str) -> Result<(), ApplicationError> {
        debug!(collection = %collection_name, "Deleting collection from in-memory index");
        if self.collections.remove(collection_name).is_some() {
            info!(collection = %collection_name, "Collection removed from index.");
            Ok(())
        } else {
            warn!(collection = %collection_name, "Attempted to delete non-existent collection index");
            // Consider if this should be an error or not. Let's return Ok.
            Ok(())
        }
    }

    #[instrument(skip(self, filters, sort, facets))]
    async fn search(
        &self,
        collection_name: &str,
        query: &str,
        filters: Option<&HashMap<String, Value>>,
        sort: &[SortBy],
        facets: &[String], // <-- Receive facets slice (Added from Step 12 design)
        offset: usize,
        limit: usize,
    ) -> Result<SearchResult, ApplicationError> {
        debug!(collection = %collection_name, query = %query, has_filters = filters.is_some(), sort_count = sort.len(), facet_count = facets.len(), offset, limit, "Searching in-memory index");

        // Query can be empty if filters are provided
        if query.is_empty() && filters.is_none() {
            return Err(ApplicationError::InvalidInput(
                "Search requires a query or filters.".to_string(),
            ));
        }
        let query_lower = query.to_lowercase();
        let has_query = !query.is_empty();

        match self.collections.get(collection_name) {
            Some(collection_data) => {
                let schema_arc = collection_data.schema.clone();

                // --- Step 1 & 2: Get Filtered Document List ---
                // Combine query matching and filtering to get the base set of documents
                let filtered_docs: Vec<Document> = collection_data
                    .documents
                    .iter()
                    .filter_map(|entry| {
                        let doc_arc = entry.value();
                        // Check query first if applicable
                        if has_query {
                            let mut query_match = false;
                            // ... (query matching logic as before) ...
                            for field_def in &schema_arc.fields {
                                if field_def.index && field_def.field_type == FieldType::Text {
                                    if let Some(value) = doc_arc.fields().get(&field_def.name) {
                                        if let Some(text) = value.as_str() {
                                            if text.to_lowercase().contains(&query_lower) {
                                                query_match = true;
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                            if !query_match {
                                return None;
                            }
                        }
                        // Then check filters if applicable
                        if let Some(filter_map) = filters {
                            if !check_doc_matches_filters(doc_arc, &schema_arc, filter_map) {
                                return None; // Filtered out
                            }
                        }
                        // If passed query and filters, clone the document
                        Some((**doc_arc).clone())
                    })
                    .collect();
                trace!(
                    count = filtered_docs.len(),
                    "Documents after query & filtering"
                );

                // --- Step 3: Calculate Facets (on filtered docs) --- <--- NEW STEP ---
                let calculated_facet_counts: FacetCounts = if !facets.is_empty() {
                    trace!(requested_facets = ?facets, "Calculating facet counts");
                    calculate_facets(&filtered_docs, facets) // Use the helper function
                } else {
                    trace!("No facets requested.");
                    HashMap::new()
                };

                // --- Step 4: Apply Sorting (on filtered docs) ---
                let mut sorted_docs = filtered_docs; // Sort the results from filtering

                if !sort.is_empty() {
                    trace!(sort_criteria = ?sort, "Applying sort criteria");
                    sorted_docs.sort_unstable_by(|a, b| {
                        // ... (sorting comparison logic remains the same) ...
                        for sort_by in sort {
                            let field_name = &sort_by.field;
                            let val_a = a.fields().get(field_name);
                            let val_b = b.fields().get(field_name);
                            let comparison = compare_option_json_values(val_a, val_b);
                            let result = match sort_by.order {
                                SortOrder::Asc => comparison,
                                SortOrder::Desc => comparison.reverse(),
                            };
                            if result != Ordering::Equal {
                                return result;
                            }
                        }
                        Ordering::Equal
                    });
                    trace!(count = sorted_docs.len(), "Documents after sorting");
                } else {
                    trace!("No sorting criteria provided.");
                }

                // --- Step 5: Get total hits count (based on list *before* pagination) ---
                let total_hits = sorted_docs.len(); // Count before pagination is applied

                // --- Step 6: Apply pagination (on sorted docs) ---
                let paginated_docs: Vec<Document> =
                    sorted_docs.into_iter().skip(offset).take(limit).collect();

                debug!(
                    collection = %collection_name,
                    query = %query,
                    has_filters = filters.is_some(),
                    sort_count = sort.len(),
                    facet_count = facets.len(), // Log facet request count
                    total_hits,
                    returned_hits = paginated_docs.len(),
                    "In-memory search finished."
                );

                // --- Step 7: Return SearchResult ---
                Ok(SearchResult {
                    documents: paginated_docs,
                    total_hits,
                    facet_counts: calculated_facet_counts, // <-- Include calculated facets
                })
            }
            None => {
                // ... (collection not found error) ...
                warn!(collection = %collection_name, "Search performed on non-existent collection index");
                Err(ApplicationError::CollectionNotFound(
                    collection_name.to_string(),
                ))
            }
        }
    }

    /// Optimized batch index for in-memory store.
    #[instrument(skip(self, documents))]
    async fn index_batch(
        &self,
        collection_name: &str,
        documents: &[Document],
    ) -> Result<(), ApplicationError> {
        debug!(collection = %collection_name, count = documents.len(), "Indexing batch directly in in-memory index");
        match self.collections.get(collection_name) {
            Some(collection_data) => {
                // Insert all documents into the collection's document map
                for doc in documents {
                    collection_data
                        .documents
                        .insert(doc.id().clone(), Arc::new(doc.clone()));
                }
                Ok(())
            }
            None => {
                warn!(collection = %collection_name, "Attempted to index batch into non-existent collection index");
                Err(ApplicationError::CollectionNotFound(
                    collection_name.to_string(),
                ))
            }
        }
    }

    /// Gets the total number of documents across all collections in the in-memory index.
    async fn get_total_document_count(&self) -> Result<usize, ApplicationError> {
        // Sum the number of documents in each collection's DashMap
        let total_count = self
            .collections
            .iter() // Iterate over collections (DashMap entries)
            .map(|collection_entry| collection_entry.value().documents.len()) // Get count for each collection
            .sum(); // Sum the counts
        Ok(total_count)
    }
}

/// Helper function to check if a document matches the provided filters.
fn check_doc_matches_filters(
    doc: &Document,
    schema: &CollectionSchema,
    filters: &HashMap<String, Value>,
) -> bool {
    for (field_name, filter_value) in filters {
        trace!(doc_id = %doc.id().as_str(), filter_field = field_name, "Applying filter");

        // Get field definition from schema
        let field_def = match schema.get_field(field_name) {
            Some(def) => def,
            None => {
                trace!(
                    filter_field = field_name,
                    "Filter field not in schema, skipping doc."
                );
                return false; // Field being filtered on doesn't exist in schema
            }
        };

        // Get the actual value from the document
        let doc_value = match doc.fields().get(field_name) {
            Some(val) => val,
            None => {
                trace!(
                    filter_field = field_name,
                    "Field not present in document, skipping doc."
                );
                return false; // Field being filtered on doesn't exist in the doc
            }
        };

        // Apply the filter based on its structure (simple value vs object with operators)
        if !match_value(doc_value, filter_value, field_def.field_type.clone()) {
            trace!(
                filter_field = field_name,
                "Filter condition not met, skipping doc."
            );
            return false; // This specific filter condition failed
        }
    }

    // If we looped through all filters and none returned false, the document matches
    trace!(doc_id = %doc.id().as_str(), "All filter conditions met.");
    true
}

/// Recursive helper to match a document value against a filter value/condition.
fn match_value(doc_value: &Value, filter_condition: &Value, field_type: FieldType) -> bool {
    match filter_condition {
        // Case 1: Filter is a simple value (String, Number, Bool) -> Check for equality
        Value::String(filter_str) => {
            doc_value
                .as_str()
                .map_or(false, |doc_str| doc_str == filter_str) // Case-sensitive equality for now
        }
        Value::Number(filter_num) => {
            // Compare numbers carefully (allow float vs int comparison if reasonable)
            doc_value.as_f64().zip(filter_num.as_f64()).map_or(false, |(d, f)| (d - f).abs() < f64::EPSILON) || // float compare
             doc_value.as_i64().zip(filter_num.as_i64()).map_or(false, |(d, f)| d == f) // int compare
        }
        Value::Bool(filter_bool) => doc_value
            .as_bool()
            .map_or(false, |doc_bool| doc_bool == *filter_bool),

        // Case 2: Filter is an object -> Check for range operators (gte, lte, gt, lt)
        Value::Object(filter_ops) => {
            // Expect numeric or potentially date types for range filters
            if field_type != FieldType::Number {
                // Extend later for dates etc.
                trace!(
                    "Range filter applied to non-numeric field type {:?}, failing match.",
                    field_type
                );
                return false;
            }

            let doc_num = match doc_value.as_f64() {
                // Use f64 for general numeric comparison
                Some(n) => n,
                None => {
                    trace!(
                        "Document value is not a valid number for range comparison, failing match."
                    );
                    return false; // Doc value isn't number, cannot compare range
                }
            };

            for (op, op_value) in filter_ops {
                let op_num = match op_value.as_f64() {
                    Some(n) => n,
                    None => {
                        trace!(
                            "Filter operator value '{}' is not numeric, failing match.",
                            op
                        );
                        return false; // Filter value for operator isn't number
                    }
                };

                match op.as_str() {
                    "gte" => {
                        if !(doc_num >= op_num) {
                            return false;
                        }
                    }
                    "lte" => {
                        if !(doc_num <= op_num) {
                            return false;
                        }
                    }
                    "gt" => {
                        if !(doc_num > op_num) {
                            return false;
                        }
                    }
                    "lt" => {
                        if !(doc_num < op_num) {
                            return false;
                        }
                    }
                    _ => {
                        trace!("Unsupported filter operator: {}", op);
                        return false; // Unknown operator
                    }
                }
            }
            true // All operators in the object matched
        }

        // Case 3: Filter is an array -> Check if doc_value is IN the array (TODO later)
        Value::Array(_filter_array) => {
            // Example: "category": ["electronics", "audio"]
            warn!("'IN' array filter not implemented yet.");
            false // Not implemented yet
        }

        // Other filter types (Null, etc.) are not handled explicitly yet
        _ => {
            trace!("Unsupported filter condition type: {:?}", filter_condition);
            false
        }
    }
}

/// Helper function to compare Option<Value> for sorting
fn compare_option_json_values(opt_a: Option<&Value>, opt_b: Option<&Value>) -> Ordering {
    match (opt_a, opt_b) {
        (Some(a), Some(b)) => compare_json_values(a, b),
        (Some(_), None) => Ordering::Greater, // Values > missing/null
        (None, Some(_)) => Ordering::Less,    // missing/null < Values
        (None, None) => Ordering::Equal,
    }
}

/// Compares two non-optional &Value based on their underlying type.
fn compare_json_values(a: &Value, b: &Value) -> Ordering {
    if let (Some(num_a), Some(num_b)) = (a.as_f64(), b.as_f64()) {
        return num_a.partial_cmp(&num_b).unwrap_or(Ordering::Equal);
    }
    if let (Some(str_a), Some(str_b)) = (a.as_str(), b.as_str()) {
        return str_a.cmp(str_b);
    }
    if let (Some(bool_a), Some(bool_b)) = (a.as_bool(), b.as_bool()) {
        return bool_a.cmp(&bool_b);
    }
    if a.is_null() && b.is_null() {
        return Ordering::Equal;
    }
    if a.is_null() {
        return Ordering::Less;
    } // nulls first
    if b.is_null() {
        return Ordering::Greater;
    }
    Ordering::Equal // Fallback for mismatched/unhandled types
}

fn calculate_facets(docs: &[Document], facet_fields: &[String]) -> FacetCounts {
    let mut facet_counts: FacetCounts = HashMap::new();
    // Use HashSet for faster checking if a field is requested for faceting
    let requested_facets: HashSet<&str> = facet_fields.iter().map(String::as_str).collect();

    if requested_facets.is_empty() {
        return facet_counts; // No facets requested
    }

    for doc in docs {
        for (field_name, doc_value) in doc.fields() {
            // Only process fields requested for faceting
            if requested_facets.contains(field_name.as_str()) {
                // Convert the document value to a string representation for the facet key
                let facet_key = match doc_value {
                    Value::String(s) => Some(s.clone()),
                    Value::Number(n) => Some(n.to_string()),
                    Value::Bool(b) => Some(b.to_string()),
                    // TODO: Handle arrays later - iterate or skip? For now, skip.
                    // Value::Array(_) => Some("[array]".to_string()), // Or skip
                    // Skip Null, Object, Array for faceting for now
                    _ => None,
                };

                if let Some(key) = facet_key {
                    // Get the map for the current field_name, or insert a new one
                    let field_map = facet_counts.entry(field_name.clone()).or_default();
                    // Increment the count for the specific value (key)
                    field_map.entry(key).and_modify(|c| *c += 1).or_insert(1);
                }
            }
        }
    }
    facet_counts
}
