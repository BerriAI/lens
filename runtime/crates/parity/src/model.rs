use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Fixture {
    pub request: RequestFixture,
    pub response: ResponseFixture,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RequestFixture {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ResponseFixture {
    pub status: u16,
    pub headers: BTreeMap<String, Value>,
    pub body: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mismatch {
    pub path: String,
    pub expected: String,
    pub actual: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReplayReport {
    pub passed: usize,
    pub failures: Vec<FixtureFailure>,
}

impl ReplayReport {
    pub fn failed(&self) -> usize {
        self.failures.len()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FixtureFailure {
    pub file_name: String,
    pub mismatches: Vec<Mismatch>,
    pub diff: String,
}
