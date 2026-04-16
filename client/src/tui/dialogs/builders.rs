//! Factory functions for constructing field editor dialogs with validation rules.

use super::FieldDialog;
use super::tabbed_field::{TabSection, TabbedFieldDialog};
use super::validation::FieldValidation;

pub(in crate::tui) const DIALOG_WIDTH_RATIO: f32 = 0.7;
pub(in crate::tui) const DIALOG_HEIGHT_RATIO: f32 = 0.6;
pub(in crate::tui) const LIST_DIALOG_WIDTH: u16 = 50;
pub(in crate::tui) const LIST_DIALOG_TALL_PADDING: u16 = 4;
pub(in crate::tui) const FIELD_DIALOG_DEFAULT_WIDTH: u16 = 60;

const GENERAL_LABELS: &[&str] = &[
    "API URL",
    "Template preset",
    "Instruct preset",
    "Reasoning preset",
    "TLS Skip Verify",
    "Debug Logging",
];
const GENERAL_BOOLEAN: &[usize] = &[4, 5];
const GENERAL_SELECTOR: &[usize] = &[1, 2, 3];

const SAMPLING_LABELS: &[&str] = &[
    "Temperature",
    "Top-K",
    "Top-P",
    "Min-P",
    "Repeat Last N",
    "Repeat Penalty",
    "Max Tokens",
];

const BACKUP_LABELS: &[&str] = &[
    "Enabled",
    "Keep All Days",
    "Keep Daily Days",
    "Keep Weekly Days",
    "Rebase Threshold %",
    "Rebase Hard Ceiling",
];
const BACKUP_BOOLEAN: &[usize] = &[0];

const SUMMARIZATION_LABELS: &[&str] = &[
    "Enabled",
    "API URL",
    "Context Size",
    "Trigger Threshold",
    "Prompt",
];
const SUMMARIZATION_BOOLEAN: &[usize] = &[0];
const SUMMARIZATION_MULTILINE: &[usize] = &[4];
const SUMMARIZATION_PLACEHOLDER: &[usize] = &[1];

const TEMPLATE_EDITOR_FIELDS: &[&str] =
    &["Name", "Story String", "Example Separator", "Chat Start"];
const TEMPLATE_EDITOR_MULTILINE: &[usize] = &[1];

const INSTRUCT_EDITOR_FIELDS: &[&str] = &[
    "Name",
    "Input Sequence",
    "Output Sequence",
    "System Sequence",
    "Input Suffix",
    "Output Suffix",
    "System Suffix",
    "Stop Sequence",
    "Separator Sequence",
    "Wrap",
    "System Same As User",
    "Seq. As Stop Strings",
];
const INSTRUCT_EDITOR_BOOLEAN: &[usize] = &[9, 10, 11];

const REASONING_EDITOR_FIELDS: &[&str] = &["Name", "Prefix", "Suffix", "Separator"];

const PERSONA_FIELDS: &[&str] = &["Name", "Persona"];
const PERSONA_MULTILINE: &[usize] = &[1];

const CHARACTER_EDITOR_FIELDS: &[&str] = &[
    "Name",
    "Description",
    "Personality",
    "Scenario",
    "First Message",
    "Examples",
    "System Prompt",
    "Post-History",
];
const CHARACTER_EDITOR_MULTILINE: &[usize] = &[1, 2, 3, 4, 5, 6, 7];

const SYSTEM_PROMPT_FIELDS: &[&str] = &["Name", "Content"];
const SYSTEM_PROMPT_MULTILINE: &[usize] = &[1];

const ENTRY_EDITOR_FIELDS: &[&str] = &[
    "Keys [OR]",
    "Content",
    "Selective",
    "Keys [AND]",
    "Constant",
    "Enabled",
    "Order",
    "Depth",
    "Case Sensitive",
];
const ENTRY_EDITOR_MULTILINE: &[usize] = &[1];
const ENTRY_EDITOR_PLACEHOLDER_FIELDS: &[usize] = &[0, 3];

pub fn open_config_editor(
    sections: Vec<Vec<String>>,
    locked: Vec<Vec<usize>>,
) -> TabbedFieldDialog<'static> {
    let [general_vals, sampling_vals, backup_vals, summarization_vals]: [Vec<String>; 4] =
        sections.try_into().expect("expected 4 section vectors");
    let [general_locked, sampling_locked, backup_locked, summarization_locked]: [Vec<usize>; 4] =
        locked.try_into().expect("expected 4 lock vectors");

    let general = TabSection {
        title: "General",
        labels: GENERAL_LABELS,
        original_values: general_vals.clone(),
        values: general_vals,
        multiline_fields: &[],
        boolean_fields: GENERAL_BOOLEAN,
        selector_fields: GENERAL_SELECTOR,
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: &[],
        placeholder_text: None,
        locked_fields: general_locked,
        validated_fields: Vec::new(),
        color_preview_fields: &[],
        selected: 0,
    };

    let sampling = TabSection {
        title: "Sampling",
        labels: SAMPLING_LABELS,
        original_values: sampling_vals.clone(),
        values: sampling_vals,
        multiline_fields: &[],
        boolean_fields: &[],
        selector_fields: &[],
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: &[],
        placeholder_text: None,
        locked_fields: sampling_locked,
        validated_fields: vec![
            (0, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (1, FieldValidation::Int { min: 1, max: 100 }),
            (2, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (3, FieldValidation::Float { min: 0.0, max: 1.0 }),
            (4, FieldValidation::Int { min: -1, max: 32767 }),
            (5, FieldValidation::Float { min: 0.0, max: 2.0 }),
            (6, FieldValidation::Int { min: -1, max: 32767 }),
        ],
        color_preview_fields: &[],
        selected: 0,
    };

    let backup = TabSection {
        title: "Backup",
        labels: BACKUP_LABELS,
        original_values: backup_vals.clone(),
        values: backup_vals,
        multiline_fields: &[],
        boolean_fields: BACKUP_BOOLEAN,
        selector_fields: &[],
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: &[],
        placeholder_text: None,
        locked_fields: backup_locked,
        validated_fields: vec![
            (1, FieldValidation::Int { min: 0, max: 3650 }),
            (2, FieldValidation::Int { min: 0, max: 3650 }),
            (3, FieldValidation::Int { min: 0, max: 3650 }),
            (4, FieldValidation::Int { min: 0, max: 100 }),
            (5, FieldValidation::Int { min: 0, max: 100 }),
        ],
        color_preview_fields: &[],
        selected: 0,
    };

    let summarization = TabSection {
        title: "Summarization",
        labels: SUMMARIZATION_LABELS,
        original_values: summarization_vals.clone(),
        values: summarization_vals,
        multiline_fields: SUMMARIZATION_MULTILINE,
        boolean_fields: SUMMARIZATION_BOOLEAN,
        selector_fields: &[],
        action_fields: &[],
        separator_fields: &[],
        placeholder_fields: SUMMARIZATION_PLACEHOLDER,
        placeholder_text: Some("(inherit main api_url)"),
        locked_fields: summarization_locked,
        validated_fields: vec![
            (2, FieldValidation::Int { min: 512, max: 131072 }),
            (3, FieldValidation::Int { min: 1, max: 100 }),
        ],
        color_preview_fields: &[],
        selected: 0,
    };

    TabbedFieldDialog::new(
        " Configuration ",
        vec![general, sampling, backup, summarization],
    )
}

pub fn open_persona_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(" Edit Persona ", PERSONA_FIELDS, values, PERSONA_MULTILINE)
        .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_character_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Character ",
        CHARACTER_EDITOR_FIELDS,
        values,
        CHARACTER_EDITOR_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_template_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Template Preset ",
        TEMPLATE_EDITOR_FIELDS,
        values,
        TEMPLATE_EDITOR_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_instruct_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Instruct Preset ",
        INSTRUCT_EDITOR_FIELDS,
        values,
        &[],
    )
    .with_boolean_fields(INSTRUCT_EDITOR_BOOLEAN)
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_reasoning_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Reasoning Preset ",
        REASONING_EDITOR_FIELDS,
        values,
        &[],
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_system_prompt_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit System Prompt ",
        SYSTEM_PROMPT_FIELDS,
        values,
        SYSTEM_PROMPT_MULTILINE,
    )
    .with_validated_fields(vec![(0, FieldValidation::MaxLen(super::MAX_NAME_LENGTH))])
}

pub fn open_entry_editor(values: Vec<String>) -> FieldDialog<'static> {
    FieldDialog::new(
        " Edit Entry ",
        ENTRY_EDITOR_FIELDS,
        values,
        ENTRY_EDITOR_MULTILINE,
    )
    .with_placeholder("keyword1, keyword2, ...", ENTRY_EDITOR_PLACEHOLDER_FIELDS)
    .with_validated_fields(vec![
        (
            6,
            FieldValidation::Int {
                min: -999,
                max: 999,
            },
        ),
        (7, FieldValidation::Int { min: 0, max: 24 }),
    ])
}

pub fn open_entry_editor_non_selective(values: Vec<String>) -> FieldDialog<'static> {
    let mut dialog = open_entry_editor(values);
    dialog.hidden_fields = vec![3];
    dialog
}
