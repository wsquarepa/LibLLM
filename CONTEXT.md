# LibLLM

A terminal chat client for the llama.cpp completions API. A conversation is a tree of messages held in encrypted local storage; characters, personas, world books, and presets shape the prompt sent to the model.

## Language

### Conversation

**Session**:
One saved conversation: its message tree, the character it is with, and its chat settings.
_Avoid_: chat, thread

**Message tree**:
The branching structure of a session's messages. Every message has one parent and may have several children.
_Avoid_: history, log

**Branch**:
One sibling among a message's children. Retrying or editing a message creates a new branch instead of replacing it.
_Avoid_: fork, alternative

**Branch path**:
The chain of messages from the head back to the root; the part of the tree that is visible and sent to the model.

**Head**:
The message currently at the end of the branch path.

**Turn prompt**:
The complete text assembled for one model request from the branch path, system prompt, character, persona, world book entries, and author's note.

**Group chat**:
A session in which several characters take turns replying.

### Prompt shaping

**Character card**:
An imported character definition (name, description, scenario, greetings) that the model speaks as.
_Avoid_: bot, agent

**Persona**:
The user's own identity as presented to the model.
_Avoid_: user profile

**System prompt**:
Standing instructions placed at the top of every turn prompt.

**World book**:
A set of entries injected into the turn prompt when their keywords appear in recent messages.
_Avoid_: lorebook

**Author's note**:
A short instruction injected at a fixed depth in the turn prompt.

**Preset**:
A named bundle of settings applied to a session. Instruct presets define the chat template, context presets define prompt assembly, and reasoning presets define how model thinking is handled.

**Regex rule**:
A pattern-and-replacement applied to message text on display or before sending.

### Files

**File reference**:
A file attached to a message by path.

**File summary**:
A model-generated condensation of a referenced file, produced in the background and substituted into the turn prompt in place of the raw file.

### Storage and safety

**Data directory**:
The per-user location holding the database, config, presets, and backups.

**Passkey**:
The secret from which the database encryption key is derived.
_Avoid_: password, passphrase

**Snapshot**:
A point-in-time backup of the database taken before a risky operation.

**Migration**:
A numbered schema upgrade step applied once when the database is opened.

**Override**:
A setting forced from the command line; the in-app config dialog shows it read-only.
_Avoid_: flag

### Interface

**Statusbar**:
The bottom bar showing version and build status on the left and keybind hints on the right.

**Status message**:
A temporary Info, Warning, or Error notice shown over the statusbar for a few seconds.

**Dialog**:
A modal editor or picker layered over the chat view.
