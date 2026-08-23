# One keybinding contract for every dialog

Every dialog handler under `crates/tui/src/dialogs/` follows the same key contract so users never relearn navigation per dialog. Diverging is a review-blocking issue; a dialog that cannot conform is recorded under Exceptions here.

| Key              | Action                                                                                       |
|------------------|----------------------------------------------------------------------------------------------|
| `Up` / `Down`    | Move field focus. Never alias to anything else.                                              |
| `Left` / `Right` | Adjust the focused field value (toggle, slider, radio cycle).                                |
| `Tab` / `BackTab`| Switch dialog tabs (when present). Never alias Down or Enter.                                |
| `Enter`          | Activate the focused field (open editor / picker / commit row).                              |
| `Space`          | Toggle a boolean field, or pick a row in multi-select lists.                                 |
| `Ctrl+S`         | Save the dialog. Equivalent to focusing and pressing `[Save]`.                               |
| `Esc`            | Close the dialog. If the dialog is dirty, push `UnsavedWarning` instead of closing directly. |

## Exceptions

- `file_picker.rs`: `Tab` descends into the selected folder or accepts the file (matches shell tab-complete UX). `Up/Down/Esc` still follow the contract.
- `paged_list.rs` (inside an active search): `Tab` commits the filter (equivalent to `Enter`). Outside search mode the contract applies.
- `set_passkey.rs`: `Tab` toggles between the two password fields (no other navigation possible in a 2-field dialog).

## Consequences

Editor dialogs that mutate persistent data track a dirty bit and route `Esc` through `unsaved_warning.rs` when dirty. The warning offers `[Save & Close] [Discard] [Cancel]` and is the only place where an ambiguous "Esc on a modified dialog" decision is made.
