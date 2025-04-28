// ./infrastructure/src/search/in_memory_index.rs
use application::{ApplicationError, Index}; // Import trait and error
use async_trait::async_trait;
use dashmap::DashMap;
use domain::{CollectionSchema, Document, DocumentId, FieldType}; // Import Schema types
use std::sync::Arc;
use tracing::{debug, error, info, instrument, warn};

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
    async fn search(
        &self,
        collection_name: &str,
        query: &str,
    ) -> Result<Vec<Document>, ApplicationError> {
        // <-- Return Vec<Document>
        debug!(collection = %collection_name, query = %query, "Searching in-memory index");

        if query.is_empty() {
            return Ok(vec![]);
        }
        let query_lower = query.to_lowercase();

        match self.collections.get(collection_name) {
            Some(collection_data) => {
                let schema = &collection_data.schema;
                // Iterate and filter, but collect the full Document now
                let results: Vec<Document> = collection_data
                    .documents
                    .iter()
                    .filter_map(|entry| {
                        let doc_arc = entry.value(); // Get the Arc<Document>

                        // Check only indexed fields of type Text for a match
                        for field_def in &schema.fields {
                            if field_def.index && field_def.field_type == FieldType::Text {
                                if let Some(value) = doc_arc.fields().get(&field_def.name) {
                                    if let Some(text) = value.as_str() {
                                        if text.to_lowercase().contains(&query_lower) {
                                            // Match found, clone the Document from the Arc and return
                                            return Some((**doc_arc).clone());
                                        }
                                    }
                                }
                            }
                        }
                        None // No match in this document
                    })
                    .collect();

                debug!(collection = %collection_name, "Found {} results for query '{}'", results.len(), query);
                Ok(results) // <-- Return the collected Vec<Document>
            }
            None => {
                warn!(collection = %collection_name, "Search performed on non-existent collection index");
                Err(ApplicationError::CollectionNotFound(
                    collection_name.to_string(),
                ))
            }
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
}
