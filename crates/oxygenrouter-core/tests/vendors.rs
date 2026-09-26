//! Vendor registry storage.
//!
//! `model_count` is the load-bearing part: the reference computes it from the
//! model registry rather than storing it (`model/vendor_meta.go:15-25`, field tag
//! `gorm:"-"`), so a model rename or delete cannot leave a stale count behind.
//! These tests pin that behaviour, because a stored counter is the obvious
//! "simpler" implementation and would drift.
//!
//! GitHub@OxygenAILab | OxygenAILab@StarsailsClover

use oxygenrouter_core::{Database, ModelMetadata, Vendor};
use chrono::Utc;

fn db() -> (Database, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("vendors.db");
    (Database::new(&path).expect("database opens"), dir)
}

fn vendor(id: &str, name: &str) -> Vendor {
    let now = Utc::now();
    Vendor {
        id: id.to_string(),
        name: name.to_string(),
        description: format!("{name} description"),
        icon: "Box".to_string(),
        status: 1,
        model_count: 0,
        created_at: now,
        updated_at: now,
    }
}

fn model(id: &str, name: &str, vendor_name: &str) -> ModelMetadata {
    let now = Utc::now();
    ModelMetadata {
        id: id.to_string(),
        model_name: name.to_string(),
        description: String::new(),
        icon: String::new(),
        tags: String::new(),
        vendor: vendor_name.to_string(),
        endpoints: Vec::new(),
        name_rule: 0,
        status: 1,
        sync_official: 1,
        created_at: now,
        updated_at: now,
    }
}

#[test]
fn a_vendor_round_trips() {
    let (db, _dir) = db();
    db.upsert_vendor(&vendor("v1", "OpenAI")).unwrap();

    let fetched = db.get_vendor("v1").unwrap().expect("vendor exists");
    assert_eq!(fetched.name, "OpenAI");
    assert_eq!(fetched.icon, "Box");
    assert_eq!(fetched.status, 1);
}

#[test]
fn the_model_count_is_derived_from_the_registry() {
    let (db, _dir) = db();
    db.upsert_vendor(&vendor("v1", "OpenAI")).unwrap();
    db.upsert_vendor(&vendor("v2", "Anthropic")).unwrap();

    db.upsert_model_metadata(&model("m1", "gpt-4o", "OpenAI")).unwrap();
    db.upsert_model_metadata(&model("m2", "gpt-4o-mini", "OpenAI")).unwrap();
    db.upsert_model_metadata(&model("m3", "claude-3-5", "Anthropic")).unwrap();

    let vendors = db.list_vendors().unwrap();
    let openai = vendors.iter().find(|v| v.name == "OpenAI").unwrap();
    let anthropic = vendors.iter().find(|v| v.name == "Anthropic").unwrap();
    assert_eq!(openai.model_count, 2);
    assert_eq!(anthropic.model_count, 1);
}

#[test]
fn deleting_a_model_lowers_the_count_without_a_second_write() {
    // The counter must be a join, not a stored column: this delete touches only
    // the model row, so a stored count would still read 1 here.
    let (db, _dir) = db();
    db.upsert_vendor(&vendor("v1", "OpenAI")).unwrap();
    db.upsert_model_metadata(&model("m1", "gpt-4o", "OpenAI")).unwrap();

    let before = db.get_vendor("v1").unwrap().unwrap();
    assert_eq!(before.model_count, 1);

    db.delete_model_metadata("m1").unwrap();

    let after = db.get_vendor("v1").unwrap().unwrap();
    assert_eq!(after.model_count, 0, "the count must follow the registry");
}

#[test]
fn a_vendor_with_no_models_counts_zero() {
    let (db, _dir) = db();
    db.upsert_vendor(&vendor("v1", "Empty")).unwrap();
    let fetched = db.get_vendor("v1").unwrap().unwrap();
    assert_eq!(fetched.model_count, 0);
}

#[test]
fn vendor_names_are_unique() {
    let (db, _dir) = db();
    db.upsert_vendor(&vendor("v1", "OpenAI")).unwrap();
    // A second row with the same name must be refused by the schema, which is
    // what lets the API answer "already exists" instead of creating a duplicate.
    let duplicate = db.upsert_vendor(&vendor("v2", "OpenAI"));
    assert!(duplicate.is_err(), "duplicate vendor names must be rejected");

    assert_eq!(db.list_vendors().unwrap().len(), 1);
}

#[test]
fn find_by_name_locates_the_row_the_api_needs_to_refuse_duplicates() {
    let (db, _dir) = db();
    db.upsert_vendor(&vendor("v1", "OpenAI")).unwrap();

    let found = db.find_vendor_by_name("OpenAI").unwrap();
    assert_eq!(found.map(|v| v.id), Some("v1".to_string()));
    assert!(db.find_vendor_by_name("Missing").unwrap().is_none());
}

#[test]
fn an_update_changes_the_row_without_creating_a_second() {
    let (db, _dir) = db();
    db.upsert_vendor(&vendor("v1", "OpenAI")).unwrap();

    let mut renamed = vendor("v1", "OpenAI Inc");
    renamed.description = "renamed".to_string();
    db.upsert_vendor(&renamed).unwrap();

    let all = db.list_vendors().unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "OpenAI Inc");
    assert_eq!(all[0].description, "renamed");
}

#[test]
fn deleting_a_vendor_removes_it() {
    let (db, _dir) = db();
    db.upsert_vendor(&vendor("v1", "OpenAI")).unwrap();
    db.delete_vendor("v1").unwrap();
    assert!(db.get_vendor("v1").unwrap().is_none());
}

#[test]
fn the_list_is_ordered_by_name() {
    let (db, _dir) = db();
    for (id, name) in [("v3", "Zhipu"), ("v1", "Anthropic"), ("v2", "OpenAI")] {
        db.upsert_vendor(&vendor(id, name)).unwrap();
    }
    let names: Vec<String> = db.list_vendors().unwrap().into_iter().map(|v| v.name).collect();
    assert_eq!(names, vec!["Anthropic", "OpenAI", "Zhipu"]);
}
