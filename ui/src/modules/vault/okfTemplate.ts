/** Quoted scalars prevent a title containing ':' or a newline from changing YAML. */
export function okfConceptTemplate(type: string, title: string, actor: string, now = new Date()): string {
  return `---\ntype: ${JSON.stringify(type)}\ntitle: ${JSON.stringify(title)}\ndescription: ""\ntags: []\nstatus: draft\ngenerated:\n  by: ${JSON.stringify(actor)}\n  at: ${now.toISOString()}\nsources: []\n---\n\n# Overview\n\n`;
}
