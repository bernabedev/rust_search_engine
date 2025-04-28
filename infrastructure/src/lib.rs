// Module declarations
pub mod persistence;
pub mod search;

// Re-export all implementations
pub use persistence::{InMemoryDocumentRepository, InMemorySchemaRepository};
pub use search::in_memory_index::InMemoryIndex;
