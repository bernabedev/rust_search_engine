// ./infrastructure/src/persistence/in_memory_repository.rs
use application::{ApplicationError, DocumentRepository, SchemaRepository}; // Added SchemaRepository trait
use async_trait::async_trait;
use dashmap::DashMap;
use domain::{CollectionSchema, Document, DocumentId};
use std::sync::Arc;
use tracing::{debug, error, instrument};

// --- Schema Repository Implementation ---

#[derive(Debug, Clone, Default)]
pub struct InMemorySchemaRepository {
    // Collection Name -> Schema
    schemas: Arc<DashMap<String, Arc<CollectionSchema>>>,
}

impl InMemorySchemaRepository {
    pub fn new() -> Self {
        Self {
            schemas: Arc::new(DashMap::new()),
        }
    }
}

#[async_trait]
impl SchemaRepository for InMemorySchemaRepository {
    #[instrument(skip(self, schema))]
    async fn save(&self, schema: &CollectionSchema) -> Result<(), ApplicationError> {
        debug!(collection = %schema.name, "Saving schema definition to in-memory store");
        // Clone schema into an Arc for storage
        self.schemas
            .insert(schema.name.clone(), Arc::new(schema.clone()));
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get(&self, name: &str) -> Result<Option<CollectionSchema>, ApplicationError> {
        debug!(collection = %name, "Getting schema definition from in-memory store");
        // Get returns a Ref, so we clone the Arc, then clone the Schema inside
        let schema = self
            .schemas
            .get(name)
            .map(|schema_ref| (**schema_ref).clone());
        Ok(schema)
    }

    #[instrument(skip(self))]
    async fn delete(&self, name: &str) -> Result<bool, ApplicationError> {
        debug!(collection = %name, "Deleting schema definition from in-memory store");
        Ok(self.schemas.remove(name).is_some())
    }

    #[instrument(skip(self))]
    async fn list(&self) -> Result<Vec<String>, ApplicationError> {
        debug!("Listing all schemas from in-memory store");
        let names = self
            .schemas
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        Ok(names)
    }
}

// --- Document Repository Implementation (Collection-Aware) ---

#[derive(Debug, Clone, Default)]
pub struct InMemoryDocumentRepository {
    // Collection Name -> (Document ID -> Document)
    store: Arc<DashMap<String, DashMap<DocumentId, Arc<Document>>>>,
}

impl InMemoryDocumentRepository {
    pub fn new() -> Self {
        Self {
            store: Arc::new(DashMap::new()),
        }
    }
}

#[async_trait]
impl DocumentRepository for InMemoryDocumentRepository {
    #[instrument(skip(self, document))]
    async fn save(
        &self,
        collection_name: &str,
        document: &Document,
    ) -> Result<(), ApplicationError> {
        debug!(collection = %collection_name, doc_id = %document.id().as_str(), "Saving document to in-memory store");
        // Get or create the inner map for the collection
        let collection_store = self
            .store
            .entry(collection_name.to_string())
            .or_insert_with(DashMap::new); // Create if doesn't exist

        // Insert the document (wrapped in Arc)
        collection_store.insert(document.id().clone(), Arc::new(document.clone()));
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get(
        &self,
        collection_name: &str,
        id: &DocumentId,
    ) -> Result<Option<Document>, ApplicationError> {
        debug!(collection = %collection_name, doc_id = %id.as_str(), "Getting document from in-memory store");
        // Find the collection's map first
        if let Some(collection_store) = self.store.get(collection_name) {
            // Then find the document within that map
            let doc = collection_store.get(id).map(|doc_ref| (**doc_ref).clone());
            Ok(doc)
        } else {
            Ok(None) // Collection doesn't exist
        }
    }

    #[instrument(skip(self))]
    async fn delete(
        &self,
        collection_name: &str,
        id: &DocumentId,
    ) -> Result<bool, ApplicationError> {
        debug!(collection = %collection_name, doc_id = %id.as_str(), "Deleting document from in-memory store");
        // Find the collection's map
        if let Some(collection_store) = self.store.get(collection_name) {
            // Remove the document if it exists
            Ok(collection_store.remove(id).is_some())
        } else {
            Ok(false) // Collection or document doesn't exist
        }
    }
    /// Optimized batch save for in-memory store.
    #[instrument(skip(self, documents))]
    async fn save_batch(
        &self,
        collection_name: &str,
        documents: &[Document],
    ) -> Result<(), ApplicationError> {
        debug!(collection = %collection_name, count = documents.len(), "Saving batch directly to in-memory store");
        // Get or create the inner map for the collection
        let collection_store = self
            .store
            .entry(collection_name.to_string())
            .or_insert_with(DashMap::new);

        // Insert all documents. DashMap operations are generally thread-safe and efficient.
        // We could potentially use `extend` if the input was consumable, but insert loop is fine.
        for doc in documents {
            collection_store.insert(doc.id().clone(), Arc::new(doc.clone()));
        }
        // In-memory operations are unlikely to fail here unless OOM.
        Ok(())
    }
}
