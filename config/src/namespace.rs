/// namespace.rs
/// Validation for some generic set of named objects in config.
use std::collections::{HashMap, HashSet};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum NamespaceError {
    #[error("Duplicate entity name in {name} namespace: {key}")]
    DuplicateName { name: String, key: String },
    #[error("Banned prefix {p} in namespace {name}.")]
    BannedPrefix { name: String, p: String },
    #[error("Cannot use keyword {kw} as name in {name} namespace")]
    Keyword { name: String, kw: String },
}

pub struct Namespace<T> {
    name: String,
    entities: HashMap<String, T>,
    banned_names: HashSet<String>,
    banned_patterns: HashSet<String>,
}

impl<T> Namespace<T> {
    pub fn new(name: String) -> Self {
        Self {
            name,
            entities: HashMap::new(),
            banned_names: HashSet::new(),
            banned_patterns: HashSet::new(),
        }
    }

    pub fn ban_prefix(&mut self, p: &str) -> Result<&mut Self, NamespaceError> {
        let p = p.to_lowercase();
        for k in self.entities.keys() {
            if *k == p {
                return Err(NamespaceError::BannedPrefix {
                    name: self.name.clone(),
                    p,
                });
            }
        }
        self.banned_patterns.insert(p.to_string());
        Ok(self)
    }

    pub fn ban_names(
        &mut self,
        names: &HashSet<impl AsRef<str>>,
    ) -> Result<&mut Self, NamespaceError> {
        for name in names {
            self.banned_names.insert(name.as_ref().to_lowercase());
        }
        for k in self.entities.keys() {
            if self.banned_names.contains(k) {
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

    pub fn add(&mut self, mut name: String, v: T) -> Result<&mut Self, NamespaceError> {
        name.make_ascii_lowercase();
        if self.entities.contains_key(&name) {
            Err(NamespaceError::DuplicateName {
                name: self.name.clone(),
                key: name,
            })
        } else if self.banned_names.contains(&name) {
            Err(NamespaceError::Keyword {
                name: self.name.clone(),
                kw: name,
            })
        } else if let Some(p) = self
            .banned_patterns
            .iter()
            .find(|prefix| name.starts_with(*prefix))
        {
            Err(NamespaceError::BannedPrefix {
                name: self.name.clone(),
                p: p.to_string(),
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
