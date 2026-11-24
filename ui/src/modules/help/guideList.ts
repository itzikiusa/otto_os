// Guide files → the ordered guide list (shared by the page's index and the
// lazy full text, modules/help/sectionsIndex.ts + sections.ts).
import { SIDEBAR_MODULES, groupLabel } from '../../lib/sidebar';
import { normalizeGroup, orderSections, parseSection, type GuideSection } from './guide';

/** A module guide whose frontmatter group is missing/unknown lands in its
 *  sidebar section. */
function sidebarGroup(id: string) {
  const def = SIDEBAR_MODULES.find((m) => m.id === id);
  return def ? normalizeGroup(groupLabel(def.group)) : null;
}

export function buildGuides(files: Record<string, string>): GuideSection[] {
  const seen = new Set<string>();
  const out: GuideSection[] = [];
  for (const [path, raw] of Object.entries(files)) {
    const s = parseSection(path, raw, sidebarGroup);
    if (!s || seen.has(s.id)) continue; // unusable or a duplicate id: first wins
    seen.add(s.id);
    out.push(s);
  }
  // Sidebar order; merged sub-views (opened from Connections, Product or
  // Scheduled Tasks, no rows of their own) read right after their parent module.
  const order = SIDEBAR_MODULES.flatMap((m) => {
    if (m.id === 'connections') return [m.id, 'database', 'brokers'];
    if (m.id === 'product') return [m.id, 'design'];
    if (m.id === 'scheduled-tasks') return [m.id, 'personal-agents'];
    return [m.id];
  });
  return orderSections(out, order);
}
