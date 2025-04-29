// ./api/src/main.rs
use axum::{
    Json,
    Router,
    extract::{Path, State}, // Added Path extractor
    http::StatusCode,
    response::{IntoResponse, Json as JsonResponse, Response}, // Use JsonResponse for clarity
    routing::{delete, get, post},                             // Added delete
};
use std::env;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info, level_filters::LevelFilter, warn};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

// Import updated application layer components
use application::{
    ApplicationError, // Base error type
    BatchResponse,
    CollectionResponse,
    // DTOs / Requests / Responses
    IndexDocumentRequest,
    // Services
    IndexingService,
    ListCollectionsResponse, // New for schema listing
    SchemaService,
    SearchRequest, // Existing modified
    SearchService,
    StatsService,
};
// Import domain types used directly in API (like CollectionSchema for POST body)
use domain::CollectionSchema;
// Import infrastructure layer implementations
use infrastructure::{InMemoryDocumentRepository, InMemoryIndex, InMemorySchemaRepository};

/// Updated application state with schema management
#[derive(Clone)]
struct AppState {
    schema_service: Arc<SchemaService>,
    indexing_service: Arc<IndexingService>,
    search_service: Arc<SearchService>,
    stats_service: Arc<StatsService>,
}

const DEFAULT_PORT: u16 = 3000;

// Application entry point
#[tokio::main]
async fn main() {
    let port = match env::var("PORT") {
        Ok(port_str) => match u16::from_str(&port_str) {
            Ok(port_num) => {
                info!("Using port {} from environment variable PORT.", port_num);
                port_num
            }
            Err(_) => {
                warn!(
                    "Invalid PORT value '{}' in environment variable. Using default port {}.",
                    port_str, DEFAULT_PORT
                );
                DEFAULT_PORT
            }
        },
        Err(_) => {
            info!(
                "PORT environment variable not set. Using default port {}.",
                DEFAULT_PORT
            );
            DEFAULT_PORT
        }
    };

    // --- Logger Initialization ---
    let filter: EnvFilter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();
    info!("Logger initialized successfully.");

    // --- Dependency Injection ---
    // 1. Create infrastructure components
    let schema_repository = Arc::new(InMemorySchemaRepository::new());
    let document_repository = Arc::new(InMemoryDocumentRepository::new());
    let index = Arc::new(InMemoryIndex::new());
    info!("In-memory infrastructure components initialized.");

    // 2. Create application services, injecting dependencies
    let schema_service = Arc::new(SchemaService::new(
        schema_repository.clone(),
        index.clone(),
        document_repository.clone(), // Pass doc repo to SchemaService as well
    ));
    let indexing_service = Arc::new(IndexingService::new(
        schema_repository.clone(), // IndexingService also needs schema repo now
        document_repository.clone(),
        index.clone(),
    ));
    let search_service = Arc::new(SearchService::new(
        schema_repository.clone(), // SearchService also needs schema repo
        index.clone(),
    ));

    let stats_service = Arc::new(StatsService::new(
        schema_repository.clone(),
        index.clone(), // TODO: Pass data path later
    ));
    info!("Application services initialized.");

    // 3. Create the application state
    let app_state = AppState {
        schema_service,
        indexing_service,
        search_service,
        stats_service,
    };
    info!("Application state created.");

    // --- API Router Definition ---
    let app = Router::new()
        .route("/health", get(health_check))
        .route("/stats", get(get_stats_handler))
        // Collection Management Endpoints
        .route("/collections", post(create_collection_handler)) // Create a new collection
        .route("/collections", get(list_collections_handler)) // List all collections
        .route("/collections/:name", get(get_collection_handler)) // Get specific collection schema
        .route("/collections/:name", delete(delete_collection_handler)) // Delete a collection
        // Document Endpoints (now collection-specific)
        .route(
            "/collections/:collection_name/documents",
            post(index_document_handler),
        ) // Index doc in collection
        .route(
            "/collections/:collection_name/documents/batch",
            post(index_batch_handler),
        ) // Index batch of docs in collection
        // .route("/collections/:collection_name/documents/:doc_id", get(get_document_handler)) // TODO: Add later
        .route(
            "/collections/:collection_name/documents/:doc_id",
            delete(delete_document_handler),
        ) // Delete doc from collection
        // Search Endpoint (now collection-specific)
        .route(
            "/collections/:collection_name/search",
            post(search_documents_handler),
        ) // Search in collection
        // Provide the application state to the handlers
        .with_state(app_state);

    info!("API routes configured.");

    // --- Server Startup ---
    // ... (no changes needed)
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Server starting on {}", addr);
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => {
            info!("Server listening on {}", addr);
            listener
        }
        Err(e) => {
            error!("Failed to bind to address {}: {}", addr, e);
            std::process::exit(1);
        }
    };
    if let Err(e) = axum::serve(listener, app.into_make_service()).await {
        error!("Server error: {}", e);
        std::process::exit(1);
    }
}

// --- API Handlers ---

async fn health_check() -> impl IntoResponse {
    info!("Health check endpoint called");
    (StatusCode::OK, "OK")
}

// --- Collection Handlers ---

/// Handler for creating a new collection (POST /collections).
async fn create_collection_handler(
    State(state): State<AppState>,
    Json(payload): Json<CollectionSchema>, // Directly deserialize into domain schema type
) -> Response {
    info!(collection_name = %payload.name, "Received request to create collection");
    let name = payload.name.clone(); // Extract name before moving payload
    match state.schema_service.create_collection(payload).await {
        Ok(_) => {
            info!("Collection created successfully via handler");
            (
                StatusCode::CREATED,
                JsonResponse(CollectionResponse {
                    name: state
                        .schema_service
                        .get_collection(&name) // Use saved name
                        .await
                        .unwrap()
                        .name,
                }),
            )
                .into_response() // Return 201 Created with name
        }
        Err(e) => {
            error!("Failed to create collection via handler: {}", e);
            map_application_error_to_response(e) // Reuse error mapping
        }
    }
}

/// Handler for listing all collections (GET /collections).
async fn list_collections_handler(State(state): State<AppState>) -> Response {
    info!("Received request to list collections");
    match state.schema_service.list_collections().await {
        Ok(names) => {
            let response = ListCollectionsResponse {
                collections: names
                    .into_iter()
                    .map(|name| CollectionResponse { name })
                    .collect(),
            };
            (StatusCode::OK, JsonResponse(response)).into_response()
        }
        Err(e) => {
            error!("Failed to list collections via handler: {}", e);
            map_application_error_to_response(e)
        }
    }
}

/// Handler for getting a collection schema (GET /collections/:name).
async fn get_collection_handler(
    State(state): State<AppState>,
    Path(name): Path<String>, // Extract name from URL path
) -> Response {
    info!(collection = %name, "Received request to get collection schema");
    match state.schema_service.get_collection(&name).await {
        Ok(schema) => {
            // Serialize the full schema back to the client
            (StatusCode::OK, JsonResponse(schema)).into_response()
        }
        Err(e) => {
            error!(collection = %name, "Failed to get collection via handler: {}", e);
            map_application_error_to_response(e)
        }
    }
}

/// Handler for deleting a collection (DELETE /collections/:name).
async fn delete_collection_handler(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    info!(collection = %name, "Received request to delete collection");
    match state.schema_service.delete_collection(&name).await {
        Ok(()) => {
            info!(collection = %name, "Collection deleted successfully via handler");
            (StatusCode::NO_CONTENT, "").into_response() // 204 No Content on success
        }
        Err(e) => {
            error!(collection = %name, "Failed to delete collection via handler: {}", e);
            map_application_error_to_response(e)
        }
    }
}

// --- Document Handlers (Collection-Aware) ---

/// Handler for indexing a document (POST /collections/:collection_name/documents).
async fn index_document_handler(
    State(state): State<AppState>,
    Path(collection_name): Path<String>, // Extract collection name from path
    Json(payload): Json<IndexDocumentRequest>, // Use the new flexible request struct
) -> Response {
    info!(collection = %collection_name, doc_id = %payload.id, "Received request to index document");
    match state
        .indexing_service
        .index_document(&collection_name, payload)
        .await
    {
        Ok(_) => {
            info!(collection = %collection_name, "Document indexed successfully via handler");
            (
                StatusCode::CREATED,
                "Document indexed successfully".to_string(),
            )
                .into_response()
        }
        Err(e) => {
            error!(collection = %collection_name, "Failed to index document via handler: {}", e);
            map_application_error_to_response(e)
        }
    }
}

async fn index_batch_handler(
    State(state): State<AppState>,
    Path(collection_name): Path<String>,
    // Directly deserialize the JSON array body into Vec<IndexDocumentRequest>
    Json(batch_payload): Json<Vec<IndexDocumentRequest>>,
) -> Response {
    let batch_size = batch_payload.len(); // Get size for logging before moving
    info!(collection = %collection_name, batch_size = batch_size, "Received request to index batch documents");

    if batch_size == 0 {
        warn!(collection = %collection_name, "Received empty batch request array.");
        // Return OK but indicate zero processed, or Bad Request? Let's go with OK.
        return (
            StatusCode::OK,
            JsonResponse(BatchResponse {
                total_processed: 0,
                successful: 0,
                failed: 0,
            }),
        )
            .into_response();
    }

    match state
        .indexing_service
        .index_batch(&collection_name, batch_payload)
        .await
    {
        Ok(batch_response) => {
            // Batch processed, even if some might have failed validation (though current logic fails entire batch on validation error)
            info!(collection = %collection_name, processed = batch_response.total_processed, successful = batch_response.successful, "Batch processed successfully via handler");
            // Use status code 200 OK or 207 Multi-Status if supporting partial success later.
            // For now, 200 OK signifies the batch request was accepted and processed (even if 0 succeeded).
            (StatusCode::OK, JsonResponse(batch_response)).into_response()
        }
        Err(e @ ApplicationError::InvalidInput(_)) => {
            // Specific handling for validation errors from the batch
            error!(collection = %collection_name, "Batch indexing failed due to validation errors: {}", e);
            map_application_error_to_response(e) // Let the helper handle 400 Bad Request
        }
        Err(e) => {
            // Handle infrastructure errors or other unexpected issues during batch processing
            error!(collection = %collection_name, "Batch indexing failed via handler: {}", e);
            map_application_error_to_response(e)
        }
    }
}

/// Handler for deleting a document (DELETE /collections/:collection_name/documents/:doc_id).
async fn delete_document_handler(
    State(state): State<AppState>,
    Path((collection_name, doc_id)): Path<(String, String)>, // Extract both parts
) -> Response {
    info!(collection = %collection_name, doc_id = %doc_id, "Received request to delete document");
    match state
        .indexing_service
        .delete_document(&collection_name, &doc_id)
        .await
    {
        Ok(()) => {
            info!(collection = %collection_name, doc_id = %doc_id, "Document deleted successfully via handler");
            (StatusCode::NO_CONTENT, "").into_response() // 204 No Content
        }
        Err(e) => {
            error!(collection = %collection_name, doc_id = %doc_id, "Failed to delete document via handler: {}", e);
            map_application_error_to_response(e)
        }
    }
}

// --- Search Handler (Collection-Aware) ---

/// Handler for searching documents (GET /collections/:collection_name/search?query=...).
async fn search_documents_handler(
    State(state): State<AppState>,
    Path(collection_name): Path<String>,
    Json(request): Json<SearchRequest>, // <-- Extract SearchRequest from JSON body
) -> Response {
    // Log filters if present
    let filters_present = request.filters.is_some();
    info!(
        collection = %collection_name,
        query = %request.query,
        has_filters = filters_present,
        limit = request.limit,
        offset = request.offset,
        "Received search request via POST"
    );

    // Call the service (logic remains the same as it takes SearchRequest)
    match state
        .search_service
        .search_documents(&collection_name, request)
        .await
    {
        Ok(response) => {
            info!(collection = %collection_name, "Search completed successfully via handler, {} total hits", response.nb_hits);
            (StatusCode::OK, JsonResponse(response)).into_response()
        }
        Err(e) => {
            error!(collection = %collection_name, "Failed to search documents via handler: {}", e);
            map_application_error_to_response(e)
        }
    }
}

async fn get_stats_handler(State(state): State<AppState>) -> Response {
    info!("Received request to get statistics");
    match state.stats_service.get_stats().await {
        Ok(stats_response) => (StatusCode::OK, JsonResponse(stats_response)).into_response(),
        Err(e) => {
            error!("Failed to get statistics via handler: {}", e);
            // Use the existing mapper, InfrastructureError should map to 500
            map_application_error_to_response(e)
        }
    }
}

/// Helper function to map ApplicationError enum to HTTP status codes and response body.
/// Now includes mappings for collection-related errors.
fn map_application_error_to_response(err: ApplicationError) -> Response {
    let (status, body) = match err {
        ApplicationError::InvalidInput(msg) => (StatusCode::BAD_REQUEST, msg),
        // --- Collection Errors ---
        ApplicationError::CollectionNotFound(name) => (
            StatusCode::NOT_FOUND,
            format!("Collection '{}' not found", name),
        ),
        ApplicationError::CollectionAlreadyExists(name) => (
            StatusCode::CONFLICT,
            format!("Collection '{}' already exists", name),
        ),
        ApplicationError::SchemaError(msg) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Schema operation failed: {}", msg),
        ),
        // --- Document/Search Errors ---
        ApplicationError::NotFound(id) => (
            StatusCode::NOT_FOUND,
            format!("Document '{}' not found", id),
        ), // Keep specific doc not found?
        ApplicationError::IndexError { collection, source } => {
            error!(collection = %collection, "Indexing error: {}", source);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Indexing failed in collection '{}'", collection),
            )
        }
        ApplicationError::SearchError { collection, source } => {
            error!(collection = %collection, "Search error: {}", source);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Search failed in collection '{}'", collection),
            )
        }
        // --- Other Errors ---
        ApplicationError::InfrastructureError(msg) => {
            error!("Underlying infrastructure error: {}", msg);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "An internal server error occurred".to_string(),
            )
        }
        ApplicationError::DomainError(domain_err) => {
            // Map domain validation errors usually to Bad Request
            warn!("Domain validation failed: {}", domain_err); // Log as warning or info
            (StatusCode::BAD_REQUEST, domain_err.to_string())
        }
    };
    (status, body).into_response() // Convert tuple to Response
}
