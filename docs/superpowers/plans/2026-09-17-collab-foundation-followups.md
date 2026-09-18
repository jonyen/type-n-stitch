# Collaboration foundation: follow-ups

Deferred findings from the per-task and whole-branch reviews of the
`collab-foundation` branch, for the realtime plan to pick up.

- Task 1: minor (deferred): db tests don't assert WAL/foreign_keys pragmas or CHECK/UNIQUE enforcement
- Task 1: minor (deferred): projects.owner_id / edit_ops.author_id FKs have no ON DELETE — user deletion unsupported anyway
- Task 2: minor (deferred): reviewer noted JS sparse-array holes vs "" padding — resolved by design: Task 8 reducer pads with '' and Task 9 deletes the localStorage path; server is the source of speakerNames
- Task 3: minor (deferred): email validation only checks '@'; password min length counts bytes not chars; register's count_users-after-insert TOCTOU could double-fire adopt_orphans under concurrent first registrations
- Task 4: minor (deferred): Role::parse(..).unwrap_or(Viewer) silently downgrades a corrupt role string (DB-controlled invariant)
- Task 5: minor (deferred): replay match on op_id ignores author (probe/no-op); coverage thin for cache-hit path and NaN ranges; submit extracts Json after ProjectAccess so malformed body 400s before 403
- Task 5: minor (deferred): fold cache insert is last-writer-wins across concurrent load_doc (self-heals on seq check); empty batch still takes the write lock
- Task 6: minor (deferred): library.rs duplicates projects::member_role query — make member_role pub(crate) and reuse
- Task 6: complete (commits be7fdfc..f95ae80, review clean, 1 parked-with-ruling)
- Task 7: minor (deferred): Login — stale error on mode switch; inputs/toggle not disabled while busy; autoFocus mount-only; `FormEvent` deprecated in @types/react (use SubmitEvent<HTMLFormElement>). All plan-inherited; fold into Task 9 or final fix wave.
- Task 8: minor (deferred): renameSpeaker reducer/fold have no bound on speaker index (server: `resize(i+1)` on a u32 could allocate huge; consider a cap like 64 in ops::validate)
- Task 9: minor (deferred): last-response-wins on rapid edits (carry forward to realtime plan: op queue); `!project || !media` dead second test
- Task 9: minor (deferred): history.state.project stale after identity-driven goHome (cosmetic extra history frame)
- Task 10: minor (deferred): env-var table lacks DATABASE_URL / ADMIN_EMAIL / ADMIN_PASSWORD rows
