/** Build playground URL for OVS Web IDE (project-scoped). Author: kejiqing */
export function ovsIdeHref(projId: number): string {
  const q = new URLSearchParams({ projId: String(projId) });
  return `/ovs/?${q.toString()}`;
}

/** OVS is only available when the project worker profile is relaxed. Author: kejiqing */
export function isOvsWorkerRelaxed(workerProfileJson?: { mode?: string } | null): boolean {
  return workerProfileJson?.mode === "relaxed";
}
