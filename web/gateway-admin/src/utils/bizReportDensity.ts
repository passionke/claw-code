/** Temporal density metrics for biz.report.delta stream. Author: kejiqing */

export type BizReportDeltaRecord = {
  seq: number;
  serverDeltaMs?: number;
  clientDeltaMs: number;
  textLen: number;
};

export type BizReportDensity = {
  eventCount: number;
  spanClientMs: number;
  charsTotal: number;
  textLenMax: number;
  largeDeltaGe200: number;
  simultaneityRatioClient: number;
  maxBucketCount1msClient: number;
  maxBucketCount16msClient: number;
  maxSameClientMsStreak: number;
  iatClientMedianMs: number;
  iatClientP95Ms: number;
};

function bucketCounts(times: number[], bucketMs: number): Map<number, number> {
  const w = Math.max(1, bucketMs);
  const m = new Map<number, number>();
  for (const t of times) {
    const b = Math.floor(t / w);
    m.set(b, (m.get(b) ?? 0) + 1);
  }
  return m;
}

function maxBucket(m: Map<number, number>): number {
  let mx = 0;
  for (const c of m.values()) if (c > mx) mx = c;
  return mx;
}

function iatStats(times: number[]): { median: number; p95: number } {
  if (times.length < 2) return { median: 0, p95: 0 };
  const iats: number[] = [];
  for (let i = 1; i < times.length; i++) iats.push(times[i] - times[i - 1]);
  iats.sort((a, b) => a - b);
  return {
    median: iats[Math.floor(iats.length / 2)] ?? 0,
    p95: iats[Math.floor(iats.length * 0.95)] ?? iats[iats.length - 1] ?? 0,
  };
}

function simultaneityRatio(times: number[]): number {
  if (times.length < 2) return 0;
  let same = 0;
  for (let i = 1; i < times.length; i++) {
    if (times[i] === times[i - 1]) same += 1;
  }
  return same / (times.length - 1);
}

export function computeBizReportDensity(
  log: BizReportDeltaRecord[],
  maxSameClientMsStreak: number
): BizReportDensity {
  const clientTimes = log.map((r) => r.clientDeltaMs);
  const chars = log.reduce((s, r) => s + r.textLen, 0);
  const span = clientTimes.length ? clientTimes[clientTimes.length - 1] - clientTimes[0] : 0;
  const iat = iatStats(clientTimes);
  return {
    eventCount: log.length,
    spanClientMs: span,
    charsTotal: chars,
    textLenMax: log.reduce((m, r) => Math.max(m, r.textLen), 0),
    largeDeltaGe200: log.filter((r) => r.textLen >= 200).length,
    simultaneityRatioClient: simultaneityRatio(clientTimes),
    maxBucketCount1msClient: maxBucket(bucketCounts(clientTimes, 1)),
    maxBucketCount16msClient: maxBucket(bucketCounts(clientTimes, 16)),
    maxSameClientMsStreak,
    iatClientMedianMs: iat.median,
    iatClientP95Ms: iat.p95,
  };
}
