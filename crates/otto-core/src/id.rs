//! ULID-based identifiers, stored and transported as strings.

/// Identifier type used across all Otto entities.
pub type Id = String;

/// Generate a new ULID identifier.
pub fn new_id() -> Id {
    ulid::Ulid::new().to_string()
}
