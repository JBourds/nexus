/// namespace.rs
/// Validation for some generic set of named objects in config.
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum NamespaceError {
    #[error("Duplicate entity name in {name} namespace: {key}")]
    DuplicateName { name: String, key: String },
    #[error("Cannot use keyword {kw} as name in {name} namespace")]
    Keyword { name: String, kw: String },
}

pub struct Namespace<T> {
    name: String,
    entities: HashMap<String, T>,
    banned: HashSet<String>,
}

impl<T> Namespace<T> {
    pub fn new(name: String) -> Self {
        Self {
            name,
            entities: HashMap::new(),
            banned: HashSet::new(),
        }
    }

    pub fn ban_names(
        &mut self,
        names: &HashSet<impl AsRef<str>>,
    ) -> Result<&mut Self, NamespaceError> {
        for name in names {
            self.banned.insert(name.as_ref().into());
        }
        for k in self.entities.keys() {
            if self.banned.contains(k) {
                return Err(NamespaceError::DuplicateName {
                    name: self.name.clone(),
                    key: k.clone(),
                });
            }
        }
        Ok(self)
    }

    pub fn add_entries(&mut self, it: HashMap<String, T>) -> Result<&mut Self, NamespaceError> {
        for (name, v) in it.into_iter() {
            self.add(name, v)?;
        }
        Ok(self)
    }

    pub fn add(&mut self, mut name: String, mut v: T) -> Result<&mut Self, NamespaceError> {
        name.make_ascii_lowercase();
        if self.entities.contains_key(&name) {
            Err(NamespaceError::DuplicateName {
                name: self.name.clone(),
                key: name,
            })
        } else if self.banned.contains(&name) {
            Err(NamespaceError::Keyword {
                name: self.name.clone(),
                kw: name,
            })
        } else {
            self.entities.insert(name, v);
            Ok(self)
        }
    }
}

impl<T> From<Namespace<T>> for HashMap<String, T> {
    fn from(value: Namespace<T>) -> Self {
        value.entities
    }
}
