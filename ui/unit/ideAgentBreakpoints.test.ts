import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  agentBreakpointPaths, agentBreakpointsOf, agentBreakpointTitle, withAgentBreakpoints, withoutAgentBreakpoints,
} from '../src/modules/ide/agentDebug/ideAgentBreakpoints.ts';
import { loadSource } from './sourceHarness.ts';

const { syncSnapshotBreakpointsToStore } = loadSource(new URL('../src/modules/ide/store/ideDebugSnapshotSync.ts', import.meta.url), {
  '../../../lib/api/client': { api: {} },
  '../../../lib/loadError': { loadErrorText: String },
  '../shell/ideProjectHelpers': { canonicalizeLocalProjectPath: (p: string) => p },
  './ideTypes': { streamIdeSse() {} },
}, {}, { relative: true });

const claude = { session_id: 's1', name: 'Claude · Fix orders' };
let n = 0;
const id = () => `bp${++n}`;
const P = '/ws/app/orders.go';

describe('agent breakpoints', () => {
  it('adds tagged breakpoints next to the person’s, and takes over one on the same line', () => {
    const mine = [{ path: P, line: 5, enabled: true }, { path: P, line: 9, enabled: false }];
    const out = withAgentBreakpoints(mine, P, [{ file: P, line: 9 }, { file: P, line: 20, condition: 'n > 2' }], claude, id);
    assert.equal(out.length, 3);
    assert.equal(out[0].agent, undefined, 'the person’s line-5 breakpoint stays theirs');
    assert.equal(out[1].agent?.session_id, 's1');
    assert.equal(out[1].enabled, true, 'a breakpoint the agent asked for is on');
    assert.equal(out[2].condition, 'n > 2');
    assert.equal(agentBreakpointTitle(out[2]), 'Set by Claude · Fix orders');
    assert.equal(agentBreakpointTitle(out[0]), null);
  });

  it('setting the same line again keeps its id', () => {
    const once = withAgentBreakpoints([], P, [{ file: P, line: 7 }], claude, id);
    const twice = withAgentBreakpoints(once, P, [{ file: P, line: 7, log_message: 'x' }], claude, id);
    assert.equal(twice.length, 1);
    assert.equal(twice[0].agent?.id, once[0].agent?.id);
  });

  it('clear-mine finds a breakpoint the debugger moved, by id and by its original line', () => {
    const map = { [P]: withAgentBreakpoints([{ path: P, line: 3, enabled: true }], P, [{ file: P, line: 10 }, { file: P, line: 14 }], claude, id) };
    // Delve bound line 10 (blank) to 11: the snapshot sync moves the store's breakpoint.
    const host = { breakpointsByPath: map };
    syncSnapshotBreakpointsToStore(host, {
      id: 'd', state: 'paused', breakpoints: [
        { path: P, line: 3, enabled: true, verified: true },
        { path: P, line: 11, enabled: true, verified: true },
        { path: P, line: 14, enabled: true, verified: true },
      ],
    } as never);
    const moved = host.breakpointsByPath[P].find((b) => b.line === 11);
    assert.ok(moved?.agent, 'the tag survives the line move');
    const mine = agentBreakpointsOf(host.breakpointsByPath, 's1');
    assert.deepEqual(mine.map((b) => [b.line, b.verified]), [[11, true], [14, true]]);

    const byOriginalLine = withoutAgentBreakpoints(host.breakpointsByPath[P], P, 's1', [{ file: P, line: 10 }]);
    assert.deepEqual(byOriginalLine.map((b) => b.line), [3, 14]);
    const byId = withoutAgentBreakpoints(host.breakpointsByPath[P], P, 's1', [{ id: mine[1].id }]);
    assert.deepEqual(byId.map((b) => b.line), [3, 11]);
    const all = withoutAgentBreakpoints(host.breakpointsByPath[P], P, 's1', 'all');
    assert.deepEqual(all.map((b) => b.line), [3], 'the person’s breakpoint is never cleared');
    assert.deepEqual(agentBreakpointPaths(host.breakpointsByPath, 's1', 'all'), [P]);
  });

  it('another agent’s breakpoints are not this agent’s', () => {
    const list = withAgentBreakpoints([], P, [{ file: P, line: 4 }], { session_id: 's2', name: 'Codex' }, id);
    assert.equal(withoutAgentBreakpoints(list, P, 's1', 'all').length, 1);
    assert.deepEqual(agentBreakpointsOf({ [P]: list }, 's1'), []);
  });
});
