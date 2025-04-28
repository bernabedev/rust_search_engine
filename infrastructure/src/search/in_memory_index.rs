// ./infrastructure/src/search/in_memory_index.rs
use application::{ApplicationError, Index, SearchResult}; // Import trait and error
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
        offset: usize, // Receive offset
        limit: usize,  // Receive limit
    ) -> Result<SearchResult, ApplicationError> {
        // <-- Return SearchResult
        debug!(collection = %collection_name, query = %query, offset = offset, limit = limit, "Searching in-memory index with pagination");

        // Basic query validation (although service layer also validates)
        if query.is_empty() {
            return Ok(SearchResult {
                documents: vec![],
                total_hits: 0,
            });
        }
        let query_lower = query.to_lowercase();

        match self.collections.get(collection_name) {
            Some(collection_data) => {
                let schema = &collection_data.schema;

                // --- Step 1: Find all matching documents (without pagination) ---
                let all_matching_docs: Vec<Document> = collection_data
                    .documents
                    .iter()
                    .filter_map(|entry| {
                        let doc_arc = entry.value();
                        for field_def in &schema.fields {
                            if field_def.index && field_def.field_type == FieldType::Text {
                                if let Some(value) = doc_arc.fields().get(&field_def.name) {
                                    if let Some(text) = value.as_str() {
                                        if text.to_lowercase().contains(&query_lower) {
                                            // Match found, clone the Document
                                            return Some((**doc_arc).clone());
                                        }
                                    }
                                }
                            }
                        }
                        None
                    })
                    .collect(); // Collect all matches first

                // --- Step 2: Get total hits count ---
                let total_hits = all_matching_docs.len();

                // --- Step 3: Apply pagination ---
                let paginated_docs: Vec<Document> = all_matching_docs
                    .into_iter() // Consume the vector
                    .skip(offset)
                    .take(limit)
                    .collect();

                debug!(
                    collection = %collection_name,
                    query = %query,
                    total_hits = total_hits,
                    returned_hits = paginated_docs.len(),
                    "In-memory search finished."
                );

                // --- Step 4: Return SearchResult ---
                Ok(SearchResult {
                    documents: paginated_docs, // The documents for the current page
                    total_hits,                // The total count before pagination
                })
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
}
