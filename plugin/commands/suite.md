---
description: Run a RunPost collection (or folder) with its tests
argument-hint: <collection> [folder]
---

Run the RunPost collection `$ARGUMENTS` with `run_collection` and `output: "summary"`. Report requests,
tests passed and failed. Only if something failed, call it again with `output: "failures"` and explain
each failure briefly.
