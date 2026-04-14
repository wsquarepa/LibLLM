// Each test binary only uses a subset of shared helpers; allow unused ones.
#[allow(dead_code)]
mod common;

use libllm::character;
use libllm::worldinfo::{self, Entry, RuntimeWorldBook};

// -- Characters (pure logic) -------------------------------------------------

#[test]
fn character_parse_old_format() {
    let json = r#"{
        "name": "Alice",
        "description": "A curious adventurer",
        "personality": "Bold and inquisitive",
        "scenario": "Lost in a forest",
        "first_mes": "Where am I?",
        "mes_example": "<START>\n{{user}}: Hello\n{{char}}: Hi there!"
    }"#;

    let card = character::parse_card_json(json).unwrap();
    assert_eq!(card.name, "Alice");
    assert_eq!(card.description, "A curious adventurer");
    assert_eq!(card.personality, "Bold and inquisitive");
    assert_eq!(card.scenario, "Lost in a forest");
    assert_eq!(card.first_mes, "Where am I?");
}

#[test]
fn character_parse_new_format() {
    let json = r#"{
        "data": {
            "name": "Alice",
            "description": "A curious adventurer",
            "personality": "Bold and inquisitive",
            "scenario": "Lost in a forest",
            "first_mes": "Where am I?"
        }
    }"#;

    let card = character::parse_card_json(json).unwrap();
    assert_eq!(card.name, "Alice");
    assert_eq!(card.description, "A curious adventurer");
    assert_eq!(card.personality, "Bold and inquisitive");
}

#[test]
fn character_slugify() {
    assert_eq!(character::slugify("Hello World"), "hello-world");
    assert_eq!(character::slugify("foo--bar"), "foo-bar");
    assert_eq!(character::slugify("  spaces  "), "spaces");
    assert_eq!(character::slugify("CamelCase"), "camelcase");
    assert_eq!(character::slugify("special!@#chars"), "special-chars");
}

#[test]
fn character_build_system_prompt() {
    let card = common::full_character();
    let prompt = character::build_system_prompt(&card, None);

    assert!(
        prompt.contains(&card.description),
        "prompt should contain description"
    );
    assert!(
        prompt.contains(&card.personality),
        "prompt should contain personality"
    );
    assert!(
        prompt.contains(&card.scenario),
        "prompt should contain scenario"
    );
}

#[test]
fn character_missing_fields() {
    let json = r#"{ "name": "Minimal", "description": "Just a desc" }"#;
    let card = character::parse_card_json(json).unwrap();

    assert_eq!(card.name, "Minimal");
    assert_eq!(card.description, "Just a desc");
    assert!(card.personality.is_empty());
    assert!(card.scenario.is_empty());
    assert!(card.first_mes.is_empty());
    assert!(card.mes_example.is_empty());
    assert!(card.system_prompt.is_empty());
    assert!(card.post_history_instructions.is_empty());
    assert!(card.alternate_greetings.is_empty());
}

// -- WorldBooks (pure logic) -------------------------------------------------

#[test]
fn worldbook_keyword_scanning() {
    let entries = vec![
        common::worldbook_entry(vec!["dragon"], "Fire creature"),
        common::worldbook_entry(vec!["elf"], "Pointy ears"),
    ];
    let wb = common::worldbook("Lore", entries);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec!["I saw a dragon in the sky"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    assert_eq!(activated.len(), 1);
    assert_eq!(activated[0].content, "Fire creature");
}

#[test]
fn worldbook_selective_entries() {
    let selective_entry = Entry {
        keys: vec!["king".to_string()],
        secondary_keys: vec!["castle".to_string()],
        selective: true,
        content: "The king lives in the castle".to_string(),
        constant: false,
        enabled: true,
        order: 0,
        depth: 4,
        case_sensitive: false,
    };
    let wb = common::worldbook("Selective", vec![selective_entry]);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let only_primary = vec!["The king walked through town"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &only_primary);
    assert!(
        activated.is_empty(),
        "should not activate with only primary key"
    );

    let both_keys = vec!["The king returned to the castle"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &both_keys);
    assert_eq!(activated.len(), 1);
    assert_eq!(activated[0].content, "The king lives in the castle");
}

#[test]
fn worldbook_constant_entries() {
    let entries = vec![
        common::constant_entry("Always present lore"),
        common::worldbook_entry(vec!["specific"], "Only when matched"),
    ];
    let wb = common::worldbook("Mixed", entries);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec!["No keywords here"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    assert_eq!(activated.len(), 1);
    assert_eq!(activated[0].content, "Always present lore");
}

#[test]
fn worldbook_disabled_entries() {
    let disabled = Entry {
        keys: vec!["trigger".to_string()],
        secondary_keys: Vec::new(),
        selective: false,
        content: "Should not appear".to_string(),
        constant: false,
        enabled: false,
        order: 0,
        depth: 4,
        case_sensitive: false,
    };
    let wb = common::worldbook("Disabled", vec![disabled]);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec!["This message contains the trigger word"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    assert_eq!(activated.len(), 1);
}

#[test]
fn worldbook_case_sensitivity() {
    let case_sensitive = Entry {
        keys: vec!["Dragon".to_string()],
        secondary_keys: Vec::new(),
        selective: false,
        content: "Case sensitive entry".to_string(),
        constant: false,
        enabled: true,
        order: 0,
        depth: 4,
        case_sensitive: true,
    };
    let case_insensitive = Entry {
        keys: vec!["Elf".to_string()],
        secondary_keys: Vec::new(),
        selective: false,
        content: "Case insensitive entry".to_string(),
        constant: false,
        enabled: true,
        order: 0,
        depth: 4,
        case_sensitive: false,
    };
    let wb = common::worldbook("Case", vec![case_sensitive, case_insensitive]);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec!["I saw a dragon and an elf"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    assert_eq!(
        activated.len(),
        1,
        "only case-insensitive should match lowercase"
    );
    assert_eq!(activated[0].content, "Case insensitive entry");
}

#[test]
fn worldbook_entry_ordering() {
    let entries = vec![
        {
            let mut e = common::worldbook_entry(vec!["a"], "Order 30");
            e.order = 30;
            e
        },
        {
            let mut e = common::worldbook_entry(vec!["a"], "Order 10");
            e.order = 10;
            e
        },
        {
            let mut e = common::worldbook_entry(vec!["a"], "Order 20");
            e.order = 20;
            e
        },
    ];
    let wb = common::worldbook("Ordered", entries);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec!["Message about a"];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    assert_eq!(activated.len(), 3);
    assert_eq!(activated[0].content, "Order 10");
    assert_eq!(activated[1].content, "Order 20");
    assert_eq!(activated[2].content, "Order 30");
}

#[test]
fn worldbook_depth_filtering() {
    let mut shallow = common::worldbook_entry(vec!["keyword"], "Shallow (depth 1)");
    shallow.depth = 1;

    let mut deep = common::worldbook_entry(vec!["keyword"], "Deep (depth 4)");
    deep.depth = 4;

    let wb = common::worldbook("Depth", vec![shallow, deep]);
    let runtime = RuntimeWorldBook::from_worldbook(&wb);

    let messages = vec![
        "Old message without keyword",
        "Another old message",
        "Still old",
        "Recent message with keyword",
    ];
    let activated = worldinfo::scan_runtime_entries(&runtime, &messages);

    assert_eq!(
        activated.len(),
        2,
        "both entries should match since keyword is in recent message"
    );

    let far_messages = vec![
        "Message with keyword here",
        "No keyword",
        "No keyword",
        "No keyword",
    ];
    let activated = worldinfo::scan_runtime_entries(&runtime, &far_messages);

    assert!(
        activated.iter().any(|a| a.content == "Deep (depth 4)"),
        "deep entry should match"
    );
}

#[test]
fn worldbook_parse_legacy_format() {
    let legacy_json = serde_json::json!({
        "entries": {
            "0": {
                "key": ["dragon", "wyrm"],
                "keysecondary": ["fire"],
                "content": "Dragons breathe fire.",
                "disable": false,
                "order": 5,
                "depth": 4
            }
        }
    });

    let wb = worldinfo::parse_worldbook_json(&legacy_json.to_string(), "test-wb").unwrap();
    assert_eq!(wb.name, "test-wb");
    assert_eq!(wb.entries.len(), 1);
    let entry = &wb.entries[0];
    assert_eq!(entry.keys, vec!["dragon", "wyrm"]);
    assert_eq!(entry.secondary_keys, vec!["fire"]);
    assert_eq!(entry.content, "Dragons breathe fire.");
    assert!(entry.enabled);
}
