---
name: Pilot runs on TM=097 only
description: Pilot uses Target Market 097 (Austria); 756.xxx Swiss rules not yet implemented; TM swap idea deferred
type: project
---

Pilot currently runs with Target Market = 097 only. The 756.xxx (Swiss) rules are not fully implemented and cannot be cleanly tested against.

**Why:** GS1 097.xxx validation rules (097.038, 097.039, 097.040, 097.020) must remain as errors (not warnings) because they prevent DRIFT before EUDAMED M2M errors. Softening them would make data entry harder for firstbase users uploading to EUDAMED. Only 097.040 is specified as 756.540 for TM=756.

**How to apply:** Keep TargetMarket as "097" in all output JSON. Do not switch to TM=756. The ~348 errors from BR-UDID-073 cascade are expected behavior under TM=097. The TM=097→756 swap idea (to bypass blocking rules) is deferred — distinguishing swissdamed vs native 756 items is unsolved.
