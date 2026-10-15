import fs from 'node:fs';
const record = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const graph = record.normalized_result.graph;
const nodes = graph.nodes.filter(n => n.id.includes('Contents/Resources/Base.lproj/MainMenu.nib'));
const byID = new Map(nodes.map(n => [n.id, n]));
const objects = new Map(nodes.map(n => [n.attributes.interface_builder_object_id, n]));
const rows = nodes.filter(n => n.attributes.class_name === 'NSMenuItem').map(n => {
  const p = n.attributes.nibValues;
  const title = objects.get(p.NSTitle?.objectID)?.attributes.nibValues;
  const edge = graph.edges.find(e => e.from === n.id && e.relation === 'target_action');
  const selector = edge && graph.edges.find(e => e.from === edge.to && e.to && byID.get(e.to)?.kind === 'objc_selector');
  return {object: n.attributes.interface_builder_object_id, title: title?.NSDev ?? '', key: p.NSKeyEquiv ?? '', modifiers: p.NSKeyEquivModMask ?? 0, selector: selector && byID.get(selector.to)?.name};
});
console.log(JSON.stringify({evidence_id:record.evidence_id, target_sha256:record.subject.digest.sha256, archive:record.normalized_result.documents.find(d=>d.relative_path==='Contents/Resources/Base.lproj/MainMenu.nib'), shortcuts:rows.filter(r=>r.key), limitations:record.limitations}, null, 2));
