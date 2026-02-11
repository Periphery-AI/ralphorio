You are running one iteration of the Ralph Loop for this repository.

Project identity:
- Name: Ralph Loop
- Goal: Build a multiplayer Factorio-like game foundation using the existing Bevy (WASM) + Cloudflare Durable Object (SQLite) system.

Inputs you must use:
- ralph/prd.json
- ralph/progress.txt

Operating constraints for this iteration:
1. Choose exactly ONE highest-priority pending PRD item (`passes: false`) and work only on that item.
2. Prefer risky architectural work before easy polish. Fail fast on unknowns.
3. Keep changes focused and production-quality. No spaghetti abstractions.
4. Run feedback loops relevant to your changes (at minimum compile/type checks; include tests if touched).
5. Update `ralph/progress.txt` with concise notes:
   - Item id completed or advanced
   - Decisions made
   - Files changed
   - Validation commands and results
6. Update `ralph/prd.json`:
   - Keep `passes` false unless the acceptance steps for the item are actually met.
7. Make a git commit for this iteration with a clear message.
8. If every PRD item is complete, output exactly `<promise>COMPLETE</promise>` in the final response.

Quality requirements:
- Preserve server-authoritative behavior where applicable.
- Keep protocol/client/server contracts coherent.
- Avoid broad rewrites unless needed for the selected item.
- Do not leak secrets or add insecure defaults.

Stop condition for this single iteration:
- End after completing one item or one coherent slice of one item, with validations and a commit.
