# Example: a Go component test, start to finish

**User:** use Otto to debug why TestFlow/TestCreateOrder fails — it's the component test.

**Agent** (reads `test/flow/flow_component_test.go` and `orders/create.go`, then):
I'll start `TestFlow/TestCreateOrder` with the `component` build tag and stop where the
request is decoded and where the order is saved. Otto will ask you to confirm the run.

```json
{"tool": "otto_ui_ide_debug_start", "arguments": {
  "target": {"kind": "test", "file": "test/flow/flow_component_test.go", "name": "TestFlow/TestCreateOrder"},
  "build_tags": ["component"],
  "breakpoints": [{"file": "orders/create.go", "line": 41}, {"file": "orders/create.go", "line": 88}],
  "db_watch": true}}
```

Result: `{"state": "running", "output_tail": ["go: downloading …"], "breakpoints": [{"file": "orders/create.go", "line": 41, "verified": true}, …]}`
(a cold build), so:

```json
{"tool": "otto_ui_ide_debug_control", "arguments": {"action": "wait"}}
```

Result: a Stop at `orders/create.go:41`, `locals` = `{"req": "CreateReq{…}", "items": "[]Item len: 0"}`.

**Agent:** Stopped at `orders/create.go:41` in `Create`: `items` is empty although the test
sends two, so the JSON tag on `CreateReq.Items` does not match the payload. Proceed continues
to the save on line 88, where DB Watch will show whether a row is written.

*(turn ends)*

**User:** proceed

**Agent:** `{"action": "continue"}` → Stop at line 88, `db`: `orders` +1 row with `total = 0`.
The order is saved with no items and a zero total, which is the failing assertion. Proceed ends the test.

**User:** ok, stop and fix it

**Agent:** `{"action": "stop"}`, then proposes renaming the tag `json:"item"` → `json:"items"`.

# Negative example (non-trigger)

**User:** why does TestCreateOrder fail?

The user did not ask for Otto's debugger: read the code and the test output as usual.
Offer "I can debug it in Otto's IDE if you want to watch it" at most once.
