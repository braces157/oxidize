//! Git 20-byte SHA-1 object identifier representation.

use crate::error::CoreError;
use std::fmt;
use std::str::FromStr;

/// A 20-byte Git object identifier (SHA-1).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct ObjectId([u8; 20]);

impl ObjectId {
    /// Zero/null object ID (20 zeroes), representing non-existent or root state.
    pub const ZERO: Self = Self([0u8; 20]);

    /// Creates an `ObjectId` from raw 20-byte slice or array.
    pub const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    /// Returns the raw 20-byte representation.
    pub const fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    /// Converts into raw 20-byte array.
    pub const fn into_bytes(self) -> [u8; 20] {
        self.0
    }

    /// Checks if this matches the null object ID.
    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 20]
    }

    /// Returns the first 2 characters as hex string for loose object directory partitioning.
    pub fn loose_dir(&self) -> String {
        format!("{:02x}", self.0[0])
    }

    /// Returns the remaining 38 characters as hex string for loose object filename.
    pub fn loose_file(&self) -> String {
        let mut s = String::with_capacity(38);
        for byte in &self.0[1..] {
            s.push_str(&format!("{:02x}", byte));
        }
        s
    }

    /// Checks if the object ID starts with the specified hex prefix.
    pub fn starts_with(&self, prefix: &str) -> bool {
        let hex = self.to_string();
        hex.starts_with(prefix)
    }

    /// Computes the Git blob SHA-1 ObjectId for raw payload bytes.
    pub fn hash_blob(data: &[u8]) -> Self {
        crate::object::Blob::hash(data)
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({})", self)
    }
}

impl FromStr for ObjectId {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.len() != 40 {
            return Err(CoreError::InvalidObjectId(format!(
                "expected 40 hex characters, got {}",
                s.len()
            )));
        }

        let mut bytes = [0u8; 20];
        for i in 0..20 {
            bytes[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
                .map_err(|_| CoreError::InvalidObjectId(s.to_string()))?;
        }

        Ok(Self(bytes))
    }
}

impl AsRef<[u8]> for ObjectId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_id() {
        let zero = ObjectId::ZERO;
        assert!(zero.is_zero());
        assert_eq!(zero.to_string(), "0000000000000000000000000000000000000000");
    }

    #[test]
    fn test_from_str_roundtrip() {
        let hex = "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391";
        let oid: ObjectId = hex.parse().unwrap();
        assert_eq!(oid.to_string(), hex);
        assert_eq!(oid.loose_dir(), "e6");
        assert_eq!(oid.loose_file(), "9de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }
}
