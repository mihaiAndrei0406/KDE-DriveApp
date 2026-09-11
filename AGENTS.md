# Local maintenance instructions

- Every user-visible feature, state, action, error, or credential flow must be
  documented in `docs/GHID_UTILIZARE.md` in the same change.
- The Romanian guide remains in Romanian, describes only implemented behavior,
  and explicitly separates current features from planned work. Public technical
  documentation and the English user guide are maintained in English.
- Backup changes must also update limits for restore, retention, automation,
  remote writes, and password handling.
- Before handoff, run checks proportional to the change and update
  `docs/VERIFICATION.md` and `docs/WORK_LOG.md` whenever results or implementation
  status change materially.
