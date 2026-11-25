import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { IdeTargetError, launchPreview, pickMainTarget, pickTestTarget } from '../src/modules/ide/agentDebug/ideAgentTarget.ts';

const F = '/ws/svc/orders/orders_test.go';
const item = (label: string, line: number, kind = 'test'): any => ({ label, line, kind, language: 'go', run_spec: { target_path: F, test_name: label } });
const suiteSrc = `package orders
type OrdersSuite struct{ suite.Suite }
func TestOrdersSuite(t *testing.T) {
\tsuite.Run(t, new(OrdersSuite))
}
func (s *OrdersSuite) TestCreate() {}
func TestTable(t *testing.T) {}
`;

describe('agent debug target', () => {
  const items = [item('TestOrdersSuite', 3), item('TestTable', 7)];

  it('a test by name is the gutter item', () => {
    const r = pickTestTarget(items, suiteSrc, 'go', 'TestTable');
    assert.equal(r.item?.line, 7);
    assert.equal(r.testName, 'TestTable');
  });

  it('a table-test case runs its parent with the full name', () => {
    const r = pickTestTarget(items, suiteSrc, 'go', 'TestTable/empty_body');
    assert.equal(r.item?.label, 'TestTable');
    assert.equal(r.testName, 'TestTable/empty_body');
  });

  it('a testify suite method runs as Runner/Method (the gutter does not list methods)', () => {
    const r = pickTestTarget(items, suiteSrc, 'go', 'TestCreate');
    assert.equal(r.item?.label, 'TestOrdersSuite');
    assert.equal(r.testName, 'TestOrdersSuite/TestCreate');
  });

  it('a suite method whose runner lives in another file asks for Runner/Method; that form launches the package', () => {
    const src = 'func (s *PaySuite) TestRefund() {}\n';
    assert.throws(() => pickTestTarget([], src, 'go', 'TestRefund'), (e: IdeTargetError) => e.code === 'not_found' && /TestRunner\/TestRefund/.test(e.message));
    const r = pickTestTarget([], src, 'go', 'TestPaySuite/TestRefund');
    assert.equal(r.item, null);
    assert.equal(r.testName, 'TestPaySuite/TestRefund');
  });

  it('an unknown test names the ones that exist', () => {
    assert.throws(() => pickTestTarget(items, suiteSrc, 'go', 'TestNope'), (e: IdeTargetError) => e.code === 'not_found' && /TestOrdersSuite, TestTable/.test(e.message));
    assert.throws(() => pickTestTarget(items, '', 'python', 'a/b'), (e: IdeTargetError) => e.code === 'not_found');
  });

  it('main needs a main', () => {
    assert.equal(pickMainTarget([item('main', 4, 'main')]).line, 4);
    assert.throws(() => pickMainTarget(items), (e: IdeTargetError) => e.code === 'not_found');
  });

  it('previews the command the person approves', () => {
    assert.equal(
      launchPreview({ language: 'go', program: '/ws/svc/test/flow/flow_component_test.go', root: '/ws/svc', test_name: 'TestFlowComponent', build_flags: ['-tags=component'] }),
      'go test -tags=component -run ^TestFlowComponent$ ./test/flow',
    );
    assert.equal(
      launchPreview({ language: 'go', program: '/ws/svc/orders/orders_test.go', root: '/ws/svc', test_name: 'TestOrdersSuite/TestCreate' }),
      'go test -run ^TestOrdersSuite$/^TestCreate$ ./orders',
    );
    // The gutter's own test name comes anchored already.
    assert.equal(launchPreview({ language: 'go', program: '/ws/svc/a/a_test.go', root: '/ws/svc', test_name: '^TestA$' }), 'go test -run ^TestA$ ./a');
    assert.equal(launchPreview({ language: 'go', program: '/ws/svc/cmd/api/main.go', root: '/ws/svc', args: ['-port', '8080'] }), 'go run ./cmd/api -port 8080');
    assert.equal(launchPreview({ language: 'python', program: '/ws/p/tests/test_a.py', root: '/ws/p', test_name: 'test_one' }), 'pytest tests/test_a.py::test_one');
  });
});
