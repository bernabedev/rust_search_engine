use async_trait::async_trait;
use domain::{CollectionSchema, Document, DocumentId, DomainError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::{collections::HashMap, time::Instant}; // For document fields map
use sysinfo::{Disks, MemoryRefreshKind, Pid, System};
use thiserror::Error;
use tracing::{debug, error, info, instrument, warn};

// --- Application Errors ---
#[derive(Error, Debug)]
pub enum ApplicationError {
    #[error("Collection not found: {0}")]
    CollectionNotFound(String), // New error
    #[error("Collection already exists: {0}")]
    CollectionAlreadyExists(String), // New error
    #[error("Document not found: {0}")]
    NotFound(String),
    #[error("Indexing failed in collection '{collection}': {source}")]
    IndexError {
        collection: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    }, // More context
    #[error("Search failed in collection '{collection}': {source}")]
    SearchError {
        collection: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    }, // More context
    #[error("Infrastructure error: {0}")]
    InfrastructureError(String), // Keep generic one
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    #[error("Domain validation error: {0}")]
    DomainError(#[from] DomainError), // Propagate domain errors cleanly
    #[error("Schema operation failed: {0}")]
    SchemaError(String), // For schema repo errors
}

// --- Infrastructure Interfaces (Traits) ---
#[derive(Debug)]
pub struct SearchResult {
    /// Documents matching the query for the requested page.
    pub documents: Vec<Document>,
    /// Total number of documents matching the query before pagination.
    pub total_hits: usize,
    // Optional: Add index processing time later if needed
    // pub index_processing_time_ms: u128,
}

#[derive(Serialize, Debug)]
pub struct MemoryStats {
    total_bytes: u64,
    used_bytes: u64,         // Physical memory used by all processes
    free_bytes: u64,         // Physical memory free
    available_bytes: u64,    // Memory available without swapping
    process_used_bytes: u64, // Memory used by this specific search engine process
}

#[derive(Serialize, Debug)]
pub struct DiskStats {
    disk_path: String, // Path of the disk being reported (e.g., where data resides)
    total_bytes: u64,
    available_bytes: u64,
}

#[derive(Serialize, Debug)]
pub struct CpuStats {
    num_cores: usize,
    load_average_one_minute: f64,
}
#[derive(Serialize, Debug)]
pub struct EngineStats {
    total_collections: usize,
    total_documents: usize,
    // index_size_bytes: u64, // TODO: Get actual size from Index trait later
}

#[derive(Serialize, Debug)]
pub struct SystemInfo {
    os_name: String,
    os_version: String,
    // kernel_version: String,
}

/// Response for the /stats endpoint.
#[derive(Serialize, Debug)]
pub struct StatsResponse {
    system_info: SystemInfo,
    memory: MemoryStats,
    // cpu: CpuStats, // Add later
    disk: DiskStats, // Reporting disk where the engine runs for now
    engine: EngineStats,
}

/// Interface for storing and retrieving Collection Schemas.
#[async_trait]
pub trait SchemaRepository: Send + Sync {
    /// Saves (creates or updates) a collection schema.
    async fn save(&self, schema: &CollectionSchema) -> Result<(), ApplicationError>;
    /// Retrieves a schema by its name.
    async fn get(&self, name: &str) -> Result<Option<CollectionSchema>, ApplicationError>;
    /// Deletes a schema by its name. Returns true if deleted.
    async fn delete(&self, name: &str) -> Result<bool, ApplicationError>;
    /// Lists the names of all existing schemas.
    async fn list(&self) -> Result<Vec<String>, ApplicationError>;
}

/// Interface for storing and retrieving documents (now collection-aware).
#[async_trait]
pub trait DocumentRepository: Send + Sync {
    /// Adds or updates a document in a specific collection.
    async fn save(
        &self,
        collection_name: &str,
        document: &Document,
    ) -> Result<(), ApplicationError>;
    /// Retrieves a document by its ID from a specific collection.
    async fn get(
        &self,
        collection_name: &str,
        id: &DocumentId,
    ) -> Result<Option<Document>, ApplicationError>;
    /// Deletes a document by its ID from a specific collection.
    async fn delete(
        &self,
        collection_name: &str,
        id: &DocumentId,
    ) -> Result<bool, ApplicationError>;
    /// Adds or updates multiple documents efficiently.
    #[instrument(skip(self, documents))]
    async fn save_batch(
        &self,
        collection_name: &str,
        documents: &[Document],
    ) -> Result<(), ApplicationError> {
        debug!(collection = %collection_name, count = documents.len(), "Saving batch via default iteration");
        // Simple default: call save sequentially for each document
        for doc in documents {
            // If one fails, should we stop or continue? Stop for now.
            self.save(collection_name, doc).await?;
        }
        Ok(())
    }
}

/// Interface for the search index (now collection-aware).
#[async_trait]
pub trait Index: Send + Sync {
    /// Ensures a collection exists in the index, configured according to the schema.
    /// Should be called after a schema is saved via SchemaRepository.
    async fn ensure_collection_exists(
        &self,
        schema: &CollectionSchema,
    ) -> Result<(), ApplicationError>;
    /// Adds or updates a document in the specified collection's index.
    async fn index_document(
        &self,
        collection_name: &str,
        document: &Document,
    ) -> Result<(), ApplicationError>;
    /// Removes a document from the specified collection's index.
    async fn delete_document(
        &self,
        collection_name: &str,
        id: &DocumentId,
    ) -> Result<(), ApplicationError>;
    /// Performs a search query against a specific collection's index.
    async fn search(
        &self,
        collection_name: &str,
        query: &str,
        offset: usize, // Now required
        limit: usize,  // Now required
    ) -> Result<SearchResult, ApplicationError>;
    /// Deletes an entire collection from the index.
    async fn delete_collection(&self, collection_name: &str) -> Result<(), ApplicationError>;
    /// Indexes multiple documents efficiently.
    #[instrument(skip(self, documents))]
    async fn index_batch(
        &self,
        collection_name: &str,
        documents: &[Document],
    ) -> Result<(), ApplicationError> {
        debug!(collection = %collection_name, count = documents.len(), "Indexing batch via default iteration");
        // Simple default: call index_document sequentially
        for doc in documents {
            self.index_document(collection_name, doc).await?;
        }
        Ok(())
    }
    /// Returns the total number of documents in the index.
    async fn get_total_document_count(&self) -> Result<usize, ApplicationError>;
}

// --- Request/Response Models (Data Transfer Objects - DTOs) ---

// Schema creation request uses the domain::CollectionSchema directly for deserialization
// No specific DTO needed here if API accepts the domain::CollectionSchema JSON shape.

#[derive(Serialize, Debug)]
pub struct CollectionResponse {
    pub name: String,
    // Maybe include fields definition as well
}

#[derive(Serialize, Debug)]
pub struct ListCollectionsResponse {
    pub collections: Vec<CollectionResponse>,
}

/// Request to index a document (flexible fields).
#[derive(Deserialize, Debug)]
pub struct IndexDocumentRequest {
    /// The ID for the document.
    pub id: String,
    /// The fields of the document as a JSON object.
    pub fields: HashMap<String, Value>,
}

#[derive(Serialize, Debug)]
pub struct BatchResponse {
    pub total_processed: usize,
    pub successful: usize,
    pub failed: usize,
}

// SearchRequest remains the same (just a query string)
#[derive(Deserialize, Debug)]
pub struct SearchRequest {
    pub query: String,
    /// Maximum number of hits to return (page size). Optional.
    #[serde(default = "default_limit")] // Provide default if missing
    pub limit: usize,
    /// Number of hits to skip (for pagination). Optional.
    #[serde(default)] // Defaults to 0 if missing
    pub offset: usize,
}

// Function to provide default limit for serde
fn default_limit() -> usize {
    20 // Default page size
}

/// Includes the ID and all stored fields.
#[derive(Serialize, Debug, Clone)] // Clone needed if we construct this before final response
pub struct SearchHit {
    pub id: String,
    pub fields: HashMap<String, Value>,
    // Maybe add score later: pub score: f32,
}

// SearchResponse remains the same (list of document IDs)
#[derive(Serialize, Debug)]
pub struct SearchResponse {
    /// Array of matching documents for the current page.
    pub hits: Vec<SearchHit>,
    /// Total number of documents matching the query.
    pub nb_hits: usize,
    /// The original search query string.
    pub query: String,
    /// The maximum number of hits returned per page.
    pub limit: usize,
    /// The number of hits skipped (offset).
    pub offset: usize,
    /// The current page number (1-based).
    pub page: usize,
    /// Total number of pages available.
    pub total_pages: usize,
    /// Time taken by the search operation in milliseconds.
    pub processing_time_ms: u128,
}

// --- Application Services (Use Cases) ---

/// Service for managing collection schemas.
pub struct SchemaService {
    schema_repo: Arc<dyn SchemaRepository>,
    index: Arc<dyn Index>, // Index needs to know about schema changes
    doc_repo: Arc<dyn DocumentRepository>, // Maybe needed if deleting collection requires cleaning repo
}

impl SchemaService {
    pub fn new(
        schema_repo: Arc<dyn SchemaRepository>,
        index: Arc<dyn Index>,
        doc_repo: Arc<dyn DocumentRepository>,
    ) -> Self {
        Self {
            schema_repo,
            index,
            doc_repo,
        }
    }

    #[instrument(skip(self, schema_def))]
    pub async fn create_collection(
        &self,
        schema_def: CollectionSchema,
    ) -> Result<(), ApplicationError> {
        let collection_name = schema_def.name.clone();
        info!(collection = %collection_name, "Attempting to create collection");

        // Validate schema using domain logic (build)
        let schema = schema_def.build()?; // Propagates DomainError via From impl

        // Check if collection already exists
        if self.schema_repo.get(&schema.name).await?.is_some() {
            warn!(collection = %schema.name, "Attempt creation failed: collection already exists");
            return Err(ApplicationError::CollectionAlreadyExists(schema.name));
        }

        // 1. Save schema definition
        self.schema_repo.save(&schema).await.map_err(|e| {
            error!(collection = %schema.name, "Failed to save schema definition: {}", e);
            ApplicationError::SchemaError(format!("Failed to save schema: {}", e))
        })?;
        info!(collection = %schema.name, "Schema definition saved successfully");

        // 2. Ensure collection exists in the search index infrastructure
        // This step allows the index implementation (e.g., Tantivy) to prepare itself.
        if let Err(e) = self.index.ensure_collection_exists(&schema).await {
            error!(collection = %schema.name, "Failed to ensure collection exists in index: {}", e);
            // Rollback? If index creation fails, should we delete the schema definition?
            // For now, log the error and return failure. Consider rollback strategies later.
            if let Err(del_err) = self.schema_repo.delete(&schema.name).await {
                error!(collection = %schema.name, "Rollback failed: Could not delete schema definition after index creation failure: {}", del_err);
            }
            return Err(e); // Return the original index error
        }
        info!(collection = %schema.name, "Collection created successfully in index infrastructure");

        // Optionally: Create corresponding storage in DocumentRepository if needed
        // E.g., create a table, namespace, etc. This depends on the repo impl.
        // For in-memory, it might not be necessary until first document save.

        Ok(())
    }

    #[instrument(skip(self))]
    pub async fn get_collection(&self, name: &str) -> Result<CollectionSchema, ApplicationError> {
        info!(collection = %name, "Attempting to retrieve collection schema");
        self.schema_repo
            .get(name)
            .await? // Handle repo error
            .ok_or_else(|| {
                warn!(collection = %name, "Collection schema not found");
                ApplicationError::CollectionNotFound(name.to_string())
            })
    }

    #[instrument(skip(self))]
    pub async fn list_collections(&self) -> Result<Vec<String>, ApplicationError> {
        info!("Attempting to list all collections");
        self.schema_repo.list().await.map_err(|e| {
            error!("Failed to list collections from repository: {}", e);
            ApplicationError::SchemaError(format!("Failed to list schemas: {}", e))
        })
    }

    #[instrument(skip(self))]
    pub async fn delete_collection(&self, name: &str) -> Result<(), ApplicationError> {
        info!(collection = %name, "Attempting to delete collection");

        // 1. Ensure collection exists before attempting deletion
        if self.schema_repo.get(name).await?.is_none() {
            warn!(collection = %name, "Deletion failed: collection not found");
            return Err(ApplicationError::CollectionNotFound(name.to_string()));
        }

        // 2. Delete from the search index first
        if let Err(e) = self.index.delete_collection(name).await {
            error!(collection = %name, "Failed to delete collection from index: {}", e);
            // Proceed to delete schema definition anyway? Or fail here? Let's fail here for now.
            return Err(e);
        }
        info!(collection = %name, "Collection deleted from index successfully");

        // 3. Delete the schema definition
        match self.schema_repo.delete(name).await {
            Ok(true) => {
                info!(collection = %name, "Schema definition deleted successfully");
                // Optionally: Trigger cleanup in DocumentRepository if needed
                // e.g., self.doc_repo.delete_collection_storage(name).await;
                Ok(())
            }
            Ok(false) => {
                // This shouldn't happen if we checked existence first, but handle defensively.
                warn!(collection = %name, "Schema definition not found during deletion, though it existed before.");
                Err(ApplicationError::CollectionNotFound(name.to_string())) // Or maybe Ok(())?
            }
            Err(e) => {
                error!(collection = %name, "Failed to delete schema definition: {}", e);
                Err(ApplicationError::SchemaError(format!(
                    "Failed to delete schema: {}",
                    e
                )))
            }
        }
    }
}

/// Service responsible for document indexing (now collection-aware).
pub struct IndexingService {
    schema_repo: Arc<dyn SchemaRepository>, // Needed to validate documents
    doc_repo: Arc<dyn DocumentRepository>,
    index: Arc<dyn Index>,
}

impl IndexingService {
    pub fn new(
        schema_repo: Arc<dyn SchemaRepository>,
        doc_repo: Arc<dyn DocumentRepository>,
        index: Arc<dyn Index>,
    ) -> Self {
        Self {
            schema_repo,
            doc_repo,
            index,
        }
    }

    #[instrument(skip(self, request), fields(collection = %collection_name, doc_id = %request.id))]
    pub async fn index_document(
        &self,
        collection_name: &str,
        request: IndexDocumentRequest, // Use the new request type
    ) -> Result<(), ApplicationError> {
        info!("Attempting to index document");

        // 1. Get the schema for the collection
        let schema = self
            .schema_repo
            .get(collection_name)
            .await?
            .ok_or_else(|| {
                warn!(collection = %collection_name, "Indexing failed: collection not found");
                ApplicationError::CollectionNotFound(collection_name.to_string())
            })?;

        // 2. Validate and create the domain Document object
        let doc_id = DocumentId::new(request.id);
        // DomainError will be converted to ApplicationError::DomainError
        let document = Document::new(doc_id.clone(), request.fields, &schema)?;
        debug!(collection = %collection_name, doc_id = %doc_id.as_str(), "Document validated against schema");

        // 3. Save to the primary repository
        if let Err(e) = self.doc_repo.save(collection_name, &document).await {
            error!(collection = %collection_name, doc_id = %doc_id.as_str(), "Failed to save document to repository: {}", e);
            // Return InfrastructureError or a more specific repo error
            return Err(ApplicationError::InfrastructureError(format!(
                "Repository save failed: {}",
                e
            )));
        }
        info!(collection = %collection_name, doc_id = %doc_id.as_str(), "Document saved to repository successfully");

        // 4. Add to the search index
        if let Err(e) = self.index.index_document(collection_name, &document).await {
            error!(collection = %collection_name, doc_id = %doc_id.as_str(), "Failed to index document: {}", e);
            // Consider rollback / compensation logic here later
            // Wrap the underlying error
            return Err(ApplicationError::IndexError {
                collection: collection_name.to_string(),
                source: Box::new(e),
            });
        }
        info!(collection = %collection_name, doc_id = %doc_id.as_str(), "Document indexed successfully");

        Ok(())
    }

    #[instrument(skip(self), fields(collection = %collection_name, doc_id = %id))]
    pub async fn delete_document(
        &self,
        collection_name: &str,
        id: &str,
    ) -> Result<(), ApplicationError> {
        info!("Attempting to delete document");
        // Optional: Check if collection exists first via schema_repo.get(collection_name)
        // ...

        let doc_id = DocumentId::new(id.to_string());

        // 1. Delete from the index
        if let Err(e) = self.index.delete_document(collection_name, &doc_id).await {
            error!(collection = %collection_name, doc_id = %id, "Failed to delete document from index: {}", e);
            // Decide if we proceed to delete from repo or stop here. Stop for now.
            return Err(ApplicationError::IndexError {
                collection: collection_name.to_string(),
                source: Box::new(e),
            });
        }
        info!(collection = %collection_name, doc_id = %id, "Document deleted from index successfully");

        // 2. Delete from the repository
        match self.doc_repo.delete(collection_name, &doc_id).await {
            Ok(deleted) => {
                if deleted {
                    info!(collection = %collection_name, doc_id = %id, "Document deleted from repository successfully");
                } else {
                    info!(collection = %collection_name, doc_id = %id, "Document not found in repository for deletion (already deleted or never existed).");
                }
                // Considered successful even if not found in repo, as desired state is achieved.
                Ok(())
            }
            Err(e) => {
                error!(collection = %collection_name, doc_id = %id, "Failed to delete document from repository: {}", e);
                // Index delete succeeded, but repo failed. Potential inconsistency. Log and report.
                Err(ApplicationError::InfrastructureError(format!(
                    "Repository delete failed: {}",
                    e
                )))
            }
        }
    }
    /// Handles indexing a batch of documents.
    /// This implementation validates all documents first. If all are valid,
    /// it attempts to save and index them in batches using the repository/index traits.
    /// It currently fails the entire batch if any infrastructure operation fails.
    #[instrument(skip(self, batch_request), fields(collection = %collection_name, batch_size = batch_request.len()))]
    pub async fn index_batch(
        &self,
        collection_name: &str,
        batch_request: Vec<IndexDocumentRequest>, // Directly use Vec
    ) -> Result<BatchResponse, ApplicationError> {
        // Return BatchResponse on success
        info!("Attempting to index batch of documents");

        if batch_request.is_empty() {
            warn!("Received an empty batch request.");
            return Ok(BatchResponse {
                total_processed: 0,
                successful: 0,
                failed: 0,
            });
        }

        // 1. Get the schema for the collection
        let schema = self
            .schema_repo
            .get(collection_name)
            .await?
            .ok_or_else(|| {
                warn!(collection = %collection_name, "Batch indexing failed: collection not found");
                ApplicationError::CollectionNotFound(collection_name.to_string())
            })?;
        let schema_arc = Arc::new(schema); // Put schema in Arc for sharing across potential concurrent validations

        // 2. Validate all documents in the batch
        let total_processed = batch_request.len();
        let mut valid_documents = Vec::with_capacity(total_processed);
        let mut validation_errors = Vec::new();

        // --- Validation ---
        // You could potentially parallelize validation using something like futures::stream::iter + map + try_collect
        // For simplicity, let's do it sequentially first.
        for (index, request) in batch_request.into_iter().enumerate() {
            let doc_id_str = request.id.clone(); // Clone ID for error reporting
            let doc_id = DocumentId::new(request.id);
            match Document::new(doc_id.clone(), request.fields, &schema_arc) {
                Ok(doc) => {
                    valid_documents.push(doc);
                }
                Err(domain_err) => {
                    let error_msg = format!(
                        "Document at index {} (ID: '{}') failed validation: {}",
                        index, doc_id_str, domain_err
                    );
                    warn!("{}", error_msg); // Log validation failure
                    validation_errors.push(error_msg);
                    // Decide on batch failure strategy. Let's collect all errors and fail if any exist.
                }
            }
        }

        // --- Handle Validation Results ---
        if !validation_errors.is_empty() {
            let combined_error_message = validation_errors.join("; ");
            error!(
                "Batch validation failed for {} documents. Errors: {}",
                validation_errors.len(),
                combined_error_message
            );
            // Return a clear input error indicating validation failure
            return Err(ApplicationError::InvalidInput(format!(
                "Batch contained {} validation errors. First error: {}",
                validation_errors.len(),
                validation_errors
                    .first()
                    .unwrap_or(&"Unknown validation error".to_string()) // Provide first error detail
            )));
        }

        // If we reach here, all documents passed validation.
        debug!(
            "All {} documents in the batch passed validation.",
            valid_documents.len()
        );

        // 3. Save the batch to the primary repository
        if let Err(e) = self
            .doc_repo
            .save_batch(collection_name, &valid_documents)
            .await
        {
            error!(collection = %collection_name, count = valid_documents.len(), "Failed to save document batch to repository: {}", e);
            // Fail the whole batch if repo save fails. Need better transactionality later.
            return Err(ApplicationError::InfrastructureError(format!(
                "Repository batch save failed: {}",
                e
            )));
        }
        info!(collection = %collection_name, count = valid_documents.len(), "Document batch saved to repository successfully");

        // 4. Add the batch to the search index
        if let Err(e) = self
            .index
            .index_batch(collection_name, &valid_documents)
            .await
        {
            error!(collection = %collection_name, count = valid_documents.len(), "Failed to index document batch: {}", e);
            // Fail the whole batch if index fails. Repo save already happened - inconsistency!
            // TODO: Implement compensation logic (e.g., attempt to delete saved docs) or use 2PC/Sagas pattern.
            return Err(ApplicationError::IndexError {
                collection: collection_name.to_string(),
                source: Box::new(e), // Assuming e implements Error + Send + Sync
            });
        }
        info!(collection = %collection_name, count = valid_documents.len(), "Document batch indexed successfully");

        // If all steps succeed
        Ok(BatchResponse {
            total_processed,
            successful: valid_documents.len(), // Should equal total_processed if no errors occurred before infra calls
            failed: 0, // We are failing the whole batch on infra error currently
        })
    }
}

/// Service responsible for search (now collection-aware).
pub struct SearchService {
    schema_repo: Arc<dyn SchemaRepository>, // Needed to check collection exists
    index: Arc<dyn Index>,
    // doc_repo: Arc<dyn DocumentRepository>, // Add later if needed to fetch full docs
}

// Sensible maximum limit to prevent abuse
const MAX_SEARCH_LIMIT: usize = 1000;
// Default limit if not specified or invalid
// const DEFAULT_SEARCH_LIMIT: usize = 20;

impl SearchService {
    pub fn new(schema_repo: Arc<dyn SchemaRepository>, index: Arc<dyn Index>) -> Self {
        Self { schema_repo, index }
    }

    #[instrument(skip(self, request), fields(collection = %collection_name, query = %request.query, limit = request.limit, offset = request.offset))]
    pub async fn search_documents(
        &self,
        collection_name: &str,
        request: SearchRequest, // Receives the updated request DTO
    ) -> Result<SearchResponse, ApplicationError> {
        info!("Attempting to search documents with pagination");

        let start_time: Instant = Instant::now(); // Start timing

        // --- Validate Input ---
        let query = request.query.trim();
        if query.is_empty() {
            return Err(ApplicationError::InvalidInput(
                "Search query cannot be empty".to_string(),
            ));
        }

        // Apply default and max limit
        let limit = request.limit.min(MAX_SEARCH_LIMIT).max(1); // Ensure limit is at least 1 and <= MAX
        let offset = request.offset;

        // --- Check Collection ---
        if self.schema_repo.get(collection_name).await?.is_none() {
            warn!(collection = %collection_name, "Search failed: collection not found");
            return Err(ApplicationError::CollectionNotFound(
                collection_name.to_string(),
            ));
        }

        // --- Perform Search ---
        match self
            .index
            .search(collection_name, query, offset, limit)
            .await
        {
            // Pass offset & limit
            Ok(search_result) => {
                // Receives SearchResult
                let duration = start_time.elapsed();
                let processing_time_ms = duration.as_millis();

                info!(
                    collection = %collection_name,
                    query = %query,
                    total_hits = search_result.total_hits,
                    returned_hits = search_result.documents.len(),
                    time_ms = processing_time_ms,
                    "Search successful"
                );

                // --- Calculate Metadata ---
                let nb_hits = search_result.total_hits;
                // Avoid division by zero for total_pages
                let total_pages = if limit == 0 {
                    0
                } else {
                    nb_hits.div_ceil(limit)
                };
                // Calculate page number (1-based)
                let page = if limit == 0 { 0 } else { (offset / limit) + 1 };

                // --- Map Documents to Hits ---
                let hits: Vec<SearchHit> = search_result
                    .documents // Use documents from SearchResult
                    .into_iter()
                    .map(|doc| SearchHit {
                        id: doc.id().clone().into(),
                        fields: doc.fields().clone(),
                    })
                    .collect();

                // --- Construct Final Response ---
                Ok(SearchResponse {
                    hits,
                    nb_hits,
                    query: query.to_string(), // Return the trimmed query used
                    limit,
                    offset,
                    page,
                    total_pages,
                    processing_time_ms,
                })
            }
            Err(e) => {
                let duration = start_time.elapsed();
                error!(
                    collection = %collection_name,
                    query = %query,
                    time_ms = duration.as_millis(),
                    "Search failed: {}", e
                );
                Err(ApplicationError::SearchError {
                    collection: collection_name.to_string(),
                    source: Box::new(e),
                })
            }
        }
    }
}

pub struct StatsService {
    schema_repo: Arc<dyn SchemaRepository>,
    index: Arc<dyn Index>,
    // data_path: PathBuf, // TODO: Inject configured data path later for accurate disk stats
}

impl StatsService {
    pub fn new(schema_repo: Arc<dyn SchemaRepository>, index: Arc<dyn Index>) -> Self {
        Self { schema_repo, index }
    }

    #[instrument(skip(self))]
    pub async fn get_stats(&self) -> Result<StatsResponse, ApplicationError> {
        info!("Gathering engine and system statistics");

        // --- Gather Engine Stats (Async Part) ---
        let collections_future = self.schema_repo.list();
        let documents_future = self.index.get_total_document_count();
        let (collections_result, documents_result) =
            tokio::join!(collections_future, documents_future);

        // Handle results... (same as before)
        let collections = collections_result.map_err(|e| {
            error!("Failed to list collections for stats: {}", e);
            ApplicationError::InfrastructureError("Failed to retrieve collection count".to_string())
        })?;
        let total_documents = documents_result.map_err(|e| {
            error!("Failed to get total document count for stats: {}", e);
            ApplicationError::InfrastructureError("Failed to retrieve document count".to_string())
        })?;

        let engine_stats = EngineStats {
            total_collections: collections.len(),
            total_documents,
        };
        debug!("Engine stats gathered: {:?}", engine_stats);

        // --- Gather System Stats (Sync Part in Blocking Task) ---
        let sys_info_result = tokio::task::spawn_blocking(move || {
            // Use new_all() to ensure processes/CPUs list is initially populated
            // Keep this instance local to the blocking task
            let mut sys = System::new_all();

            // Refresh specific data needed
            sys.refresh_memory_specifics(MemoryRefreshKind::everything());
            // sys.refresh_cpu_usage(); // Only if CPU stats are needed later
            // No need to refresh processes list if new_all() was used, but refresh process data if needed
            // sys.refresh_process(Pid::current().unwrap()); // Refresh current process data specifically if needed

            // Get Disks information using the separate Disks struct
            let disks = Disks::new_with_refreshed_list(); // Refreshes list and stats

            // --- Memory Stats ---
            // Get current PID using std::process::id()
            let current_pid = Pid::from(std::process::id() as usize);
            let process_memory = sys
                .process(current_pid) // Use Pid::current() result
                .map_or(0, |p| p.memory());

            let memory_stats = MemoryStats {
                total_bytes: sys.total_memory(),
                used_bytes: sys.used_memory(),
                free_bytes: sys.free_memory(),
                available_bytes: sys.available_memory(),
                process_used_bytes: process_memory,
            };

            // --- Disk Stats ---
            let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
            let mut current_disk_stats = DiskStats {
                disk_path: "unknown".to_string(),
                total_bytes: 0,
                available_bytes: 0,
            };
            let mut best_match_len = 0;

            // Iterate over the disks obtained from the Disks struct
            for disk in &disks {
                // <-- Iterate over the `disks` instance
                let mount_point = disk.mount_point();
                if current_dir.starts_with(mount_point) {
                    let mount_point_len = mount_point.as_os_str().len();
                    if mount_point_len > best_match_len {
                        best_match_len = mount_point_len;
                        current_disk_stats = DiskStats {
                            disk_path: mount_point.to_string_lossy().into_owned(),
                            total_bytes: disk.total_space(),
                            available_bytes: disk.available_space(),
                        };
                    }
                }
            }

            // --- System Info ---
            // These are static methods, no refresh needed on the instance
            let system_info = SystemInfo {
                os_name: System::name().unwrap_or_else(|| "Unknown OS".to_string()),
                os_version: System::os_version().unwrap_or_else(|| "Unknown Version".to_string()),
            };

            // Return collected stats
            Ok::<_, ApplicationError>((system_info, memory_stats, current_disk_stats))
        })
        .await
        .map_err(|e| {
            ApplicationError::InfrastructureError(format!(
                "System stat gathering task failed: {}",
                e // JoinError
            ))
        })??; // Handle JoinError and inner Result<..., ApplicationError>

        let (system_info, memory_stats, disk_stats) = sys_info_result;
        debug!(
            "System stats gathered: {:?}, {:?}, {:?}",
            system_info, memory_stats, disk_stats
        );

        // --- Construct Final Response ---
        Ok(StatsResponse {
            system_info,
            memory: memory_stats,
            disk: disk_stats,
            engine: engine_stats,
        })
    }
}
