use serde::{Deserialize, Serialize}; // For schema definition & document fields
use serde_json::Value; // To represent arbitrary document fields
use std::collections::{HashMap, HashSet};
use thiserror::Error; // For domain-specific errors

// --- Domain Errors ---
#[derive(Error, Debug, PartialEq)]
pub enum DomainError {
    #[error("Invalid schema: {0}")]
    InvalidSchema(String),
    #[error("Invalid field value for field '{field}': {reason}")]
    InvalidFieldValue { field: String, reason: String },
    #[error("Field '{0}' not found in schema")]
    FieldNotFound(String),
    #[error("Missing required field '{0}'")]
    MissingField(String), // Example validation
}

// --- Document ID (remains the same) ---
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocumentId(String);
// ... (impls for DocumentId: new, as_str, From<String>, From<DocumentId> remain the same)
impl DocumentId {
    pub fn new(id: String) -> Self {
        Self(id)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl From<String> for DocumentId {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}
impl From<DocumentId> for String {
    fn from(doc_id: DocumentId) -> Self {
        doc_id.0
    }
}

// --- Schema Definition ---

/// Defines the type of a field in a collection schema.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "lowercase")] // Allows "text", "number" in JSON
pub enum FieldType {
    Text,
    Number,
    // Add other types later: Boolean, Geo, Array, Object etc.
}

/// Defines a single field within a collection schema.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct FieldDefinition {
    pub name: String,
    #[serde(rename = "type")] // Map 'type' JSON key to 'field_type' field
    pub field_type: FieldType,
    /// Should this field be indexed for searching?
    #[serde(default)] // false if not present in JSON
    pub index: bool,
    /// Is this field required? (Example validation)
    #[serde(default)]
    pub optional: bool,
    // Add other flags later: facet, sortable, etc.
}

/// Represents the schema for a collection (index).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct CollectionSchema {
    /// The unique name of the collection. Should follow specific naming rules (e.g., alphanumeric).
    pub name: String,
    pub fields: Vec<FieldDefinition>,

    // Internal cache for faster lookups
    #[serde(skip)] // Don't serialize/deserialize this helper field
    field_lookup: Option<HashMap<String, FieldDefinition>>,
}

impl Default for CollectionSchema {
    fn default() -> Self {
        CollectionSchema {
            name: String::new(),
            fields: Vec::new(),
            field_lookup: None,
        }
    }
}

impl CollectionSchema {
    /// Validates the schema and precomputes the lookup map.
    pub fn build(mut self) -> Result<Self, DomainError> {
        // Basic validation
        if self.name.trim().is_empty()
            || !self
                .name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return Err(DomainError::InvalidSchema(
                "Collection name must be non-empty and contain only ASCII alphanumeric characters or underscores.".to_string()
            ));
        }
        if self.fields.is_empty() {
            return Err(DomainError::InvalidSchema(
                "Schema must contain at least one field.".to_string(),
            ));
        }

        let mut field_names = HashSet::new();
        let mut lookup = HashMap::new();
        for field in &self.fields {
            if field.name.trim().is_empty() {
                return Err(DomainError::InvalidSchema(
                    "Field names cannot be empty.".to_string(),
                ));
            }
            if !field_names.insert(field.name.clone()) {
                return Err(DomainError::InvalidSchema(format!(
                    "Duplicate field name found: '{}'",
                    field.name
                )));
            }
            // Ensure 'id' field is not defined manually (reserved)
            if field.name == "id" {
                return Err(DomainError::InvalidSchema(
                    "'id' is a reserved field name and cannot be defined in the schema."
                        .to_string(),
                ));
            }
            lookup.insert(field.name.clone(), field.clone());
        }

        self.field_lookup = Some(lookup);
        Ok(self)
    }

    /// Gets a field definition by name. Uses the precomputed lookup.
    pub fn get_field(&self, name: &str) -> Option<&FieldDefinition> {
        // Ensure lookup is built (should always be Some after build())
        self.field_lookup
            .as_ref()
            .and_then(|lookup| lookup.get(name))
    }

    /// Provides access to the internal field lookup map. Panics if build() wasn't called.
    fn lookup(&self) -> &HashMap<String, FieldDefinition> {
        self.field_lookup
            .as_ref()
            .expect("Schema lookup not built. Call build() first.")
    }
}

// --- Updated Document Structure ---

/// Represents a document with arbitrary fields, conforming to a schema.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Document {
    id: DocumentId,
    /// Document data stored as field name -> JSON Value pairs.
    fields: HashMap<String, Value>,
}

impl Document {
    /// Creates a new document, validating fields against the provided schema.
    pub fn new(
        id: DocumentId,
        fields: HashMap<String, Value>,
        schema: &CollectionSchema,
    ) -> Result<Self, DomainError> {
        let mut validated_fields = HashMap::new();
        let schema_lookup = schema.lookup();

        // Validate provided fields against the schema
        for (name, value) in fields {
            match schema_lookup.get(&name) {
                Some(field_def) => {
                    // Check type compatibility (basic example)
                    match field_def.field_type {
                        FieldType::Text => {
                            if !value.is_string() {
                                return Err(DomainError::InvalidFieldValue {
                                    field: name,
                                    reason: format!("Expected a text string, got {:?}", value),
                                });
                            }
                        }
                        FieldType::Number => {
                            if !value.is_number() {
                                return Err(DomainError::InvalidFieldValue {
                                    field: name,
                                    reason: format!("Expected a number, got {:?}", value),
                                });
                            }
                        } // Add checks for other types here...
                    }
                    validated_fields.insert(name, value);
                }
                None => {
                    // Field provided in data but not defined in schema
                    // Decide whether to ignore, error out, or store anyway.
                    // Let's error out for stricter adherence.
                    return Err(DomainError::FieldNotFound(name));
                }
            }
        }

        // Check for missing non-optional fields
        for (name, field_def) in schema_lookup {
            if !field_def.optional && !validated_fields.contains_key(name) {
                return Err(DomainError::MissingField(name.clone()));
            }
        }

        Ok(Self {
            id,
            fields: validated_fields,
        })
    }

    pub fn id(&self) -> &DocumentId {
        &self.id
    }

    pub fn fields(&self) -> &HashMap<String, Value> {
        &self.fields
    }

    /// Gets a specific field's value.
    pub fn get_field_value(&self, field_name: &str) -> Option<&Value> {
        self.fields.get(field_name)
    }
}

// --- Tests (add tests for schema validation and document creation/validation) ---
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json; // For creating test Values

    fn create_test_schema() -> CollectionSchema {
        CollectionSchema {
            name: "products".to_string(),
            fields: vec![
                FieldDefinition {
                    name: "name".to_string(),
                    field_type: FieldType::Text,
                    index: true,
                    optional: false,
                },
                FieldDefinition {
                    name: "description".to_string(),
                    field_type: FieldType::Text,
                    index: true,
                    optional: true,
                },
                FieldDefinition {
                    name: "price".to_string(),
                    field_type: FieldType::Number,
                    index: true,
                    optional: false,
                },
                FieldDefinition {
                    name: "stock".to_string(),
                    field_type: FieldType::Number,
                    index: false,
                    optional: true,
                }, // Not indexed
            ],
            field_lookup: None, // Will be built
        }
        .build()
        .expect("Failed to build test schema")
    }

    #[test]
    fn schema_build_success() {
        let schema = create_test_schema();
        assert_eq!(schema.name, "products");
        assert!(schema.get_field("name").is_some());
        assert!(schema.get_field("price").is_some());
        assert!(schema.get_field("nonexistent").is_none());
        assert_eq!(
            schema.get_field("name").unwrap().field_type,
            FieldType::Text
        );
        assert!(schema.get_field("name").unwrap().index);
        assert!(!schema.get_field("stock").unwrap().index);
        assert!(!schema.get_field("name").unwrap().optional);
        assert!(schema.get_field("description").unwrap().optional);
    }

    #[test]
    fn schema_build_fails_duplicate_field() {
        let result = CollectionSchema {
            name: "test".to_string(),
            fields: vec![
                FieldDefinition {
                    name: "field1".to_string(),
                    field_type: FieldType::Text,
                    index: true,
                    optional: false,
                },
                FieldDefinition {
                    name: "field1".to_string(),
                    field_type: FieldType::Number,
                    index: false,
                    optional: false,
                },
            ],
            field_lookup: None,
        }
        .build();
        assert!(
            matches!(result, Err(DomainError::InvalidSchema(msg)) if msg.contains("Duplicate field name"))
        );
    }

    #[test]
    fn schema_build_fails_invalid_name() {
        let result = CollectionSchema {
            name: "invalid-name!".to_string(),
            fields: vec![ /* ... */ ],
            field_lookup: None,
        }
        .build();
        assert!(
            matches!(result, Err(DomainError::InvalidSchema(msg)) if msg.contains("Collection name"))
        );
    }

    #[test]
    fn schema_build_fails_reserved_id_field() {
        let result = CollectionSchema {
            name: "test".to_string(),
            fields: vec![FieldDefinition {
                name: "id".to_string(),
                field_type: FieldType::Text,
                index: true,
                optional: false,
            }],
            field_lookup: None,
        }
        .build();
        assert!(
            matches!(result, Err(DomainError::InvalidSchema(msg)) if msg.contains("'id' is a reserved"))
        );
    }

    #[test]
    fn document_creation_success() {
        let schema = create_test_schema();
        let id = DocumentId::new("prod1".to_string());
        let fields: HashMap<String, Value> = [
            ("name".to_string(), json!("Test Product")),
            ("description".to_string(), json!("A great product")),
            ("price".to_string(), json!(99.99)),
            // "stock" is optional
        ]
        .iter()
        .cloned()
        .collect();

        let doc_result = Document::new(id.clone(), fields.clone(), &schema);
        assert!(doc_result.is_ok());
        let doc = doc_result.unwrap();
        assert_eq!(doc.id(), &id);
        assert_eq!(doc.fields().len(), 3); // description, name, price
        assert_eq!(doc.get_field_value("name").unwrap(), &json!("Test Product"));
    }

    #[test]
    fn document_creation_fails_missing_required_field() {
        let schema = create_test_schema();
        let id = DocumentId::new("prod2".to_string());
        // Missing required "price" field
        let fields: HashMap<String, Value> = [("name".to_string(), json!("Another Product"))]
            .iter()
            .cloned()
            .collect();

        let doc_result = Document::new(id, fields, &schema);
        assert!(matches!(doc_result, Err(DomainError::MissingField(field)) if field == "price"));
    }

    #[test]
    fn document_creation_fails_wrong_field_type() {
        let schema = create_test_schema();
        let id = DocumentId::new("prod3".to_string());
        let fields: HashMap<String, Value> = [
            ("name".to_string(), json!("Product 3")),
            ("price".to_string(), json!("expensive")), // Price should be number
        ]
        .iter()
        .cloned()
        .collect();

        let doc_result = Document::new(id, fields, &schema);
        assert!(
            matches!(doc_result, Err(DomainError::InvalidFieldValue { field, .. }) if field == "price")
        );
    }

    #[test]
    fn document_creation_fails_field_not_in_schema() {
        let schema = create_test_schema();
        let id = DocumentId::new("prod4".to_string());
        let fields: HashMap<String, Value> = [
            ("name".to_string(), json!("Product 4")),
            ("price".to_string(), json!(10.0)),
            ("extra_field".to_string(), json!("not allowed")), // Not in schema
        ]
        .iter()
        .cloned()
        .collect();

        let doc_result = Document::new(id, fields, &schema);
        assert!(
            matches!(doc_result, Err(DomainError::FieldNotFound(field)) if field == "extra_field")
        );
    }
}
