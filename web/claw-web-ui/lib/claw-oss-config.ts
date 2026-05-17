/** Central OSS (S3-compatible) config from env. Author: kejiqing */

export type ClawOssEnvConfig = {
  endpoint: string;
  bucket: string;
  region: string;
  accessKeyId: string;
  secretAccessKey: string;
  forcePathStyle: boolean;
};

export function ossEnabledFromEnv(): boolean {
  return readOssEnvConfig() != null;
}

/** Returns null when endpoint or bucket missing. Author: kejiqing */
export function readOssEnvConfig(): ClawOssEnvConfig | null {
  const endpoint = trimEnv("CLAW_OSS_ENDPOINT");
  const bucket = trimEnv("CLAW_OSS_BUCKET");
  if (!endpoint || !bucket) return null;
  const accessKeyId = trimEnv("CLAW_OSS_ACCESS_KEY_ID") ?? trimEnv("CLAW_OSS_ACCESS_KEY") ?? "";
  const secretAccessKey =
    trimEnv("CLAW_OSS_SECRET_ACCESS_KEY") ?? trimEnv("CLAW_OSS_SECRET_KEY") ?? "";
  return {
    endpoint,
    bucket,
    region: trimEnv("CLAW_OSS_REGION") ?? "us-east-1",
    accessKeyId,
    secretAccessKey,
    forcePathStyle: truthyEnv("CLAW_OSS_FORCE_PATH_STYLE"),
  };
}

function trimEnv(key: string): string | null {
  const v = process.env[key]?.trim();
  return v && v.length > 0 ? v : null;
}

function truthyEnv(key: string): boolean {
  const v = process.env[key]?.trim().toLowerCase();
  return v === "1" || v === "true" || v === "yes" || v === "on";
}
