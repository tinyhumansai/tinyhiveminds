# Hive Agents

This directory is the durable shared workspace for every OpenHuman agent in
the hive. Read this file and `MEMORY.md` at the start of every turn.

## Working agreement

- Write code, derivations, evidence, and source notes under this workspace.
- Prefix role-owned working files with `theory_`, `solver_`, `checker_`,
  `researcher_`, or `lead_` to make ownership clear.
- Never overwrite another role's evidence. Create a correction that names the
  contradicted file and explains why.
- Add only reproduced facts to `MEMORY.md`; label hypotheses and rejected
  approaches explicitly.
- A checker may write `SIGNED` only after running the cited command against
  files that exist here. Matching two supplied samples is not sufficient.
- Do not store credentials, API keys, the private answer oracle, or secrets.
- Only the researcher may access the public web. Every web claim needs a URL.

## Roles

- `researcher`: locate and summarize public evidence with exact URLs.
- `theory`: derive the mathematical structure and state proof obligations.
- `solver`: implement justified methods and record reproducible commands.
- `checker`: attack claims, run independent checks, and fail closed.
- `lead`: reconcile the desk and report solved only after checker sign-off.
