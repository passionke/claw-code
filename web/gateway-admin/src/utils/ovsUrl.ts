/** Build playground URL for OVS Web IDE (project-scoped). Author: kejiqing */
export function ovsIdeHref(projId: number): string {
  const q = new URLSearchParams({ projId: String(projId) });
  return `/ovs/?${q.toString()}`;
}
