# Wire fixture

`full.json` exercises runtime, profiles, permissions, seats, hive membership,
workflow, context, memory and reference-only credentials. Tests round-trip the
fully defaulted canonical value and validate it without network access.

The neighboring `mod.rs` embeds this committed fixture with
`include_str!("fixtures/full.json")`.
