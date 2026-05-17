/** S3-compatible OSS client (server-only). Author: kejiqing */

import {
  HeadBucketCommand,
  PutObjectCommand,
  S3Client,
} from "@aws-sdk/client-s3";
import { readOssEnvConfig } from "@/lib/claw-oss-config";

let client: S3Client | null = null;

export function getOssClient(): S3Client {
  const cfg = readOssEnvConfig();
  if (!cfg) {
    throw new Error("CLAW_OSS_ENDPOINT and CLAW_OSS_BUCKET are required");
  }
  if (!client) {
    client = new S3Client({
      endpoint: cfg.endpoint,
      region: cfg.region,
      forcePathStyle: cfg.forcePathStyle,
      credentials:
        cfg.accessKeyId && cfg.secretAccessKey
          ? { accessKeyId: cfg.accessKeyId, secretAccessKey: cfg.secretAccessKey }
          : undefined,
    });
  }
  return client;
}

export async function ossHealthCheck(): Promise<{ ok: boolean; detail?: string }> {
  const cfg = readOssEnvConfig();
  if (!cfg) return { ok: false, detail: "oss not configured" };
  try {
    await getOssClient().send(new HeadBucketCommand({ Bucket: cfg.bucket }));
    return { ok: true };
  } catch (e) {
    return { ok: false, detail: e instanceof Error ? e.message : String(e) };
  }
}

export async function putOssObject(
  key: string,
  body: string,
  contentType = "application/octet-stream",
): Promise<void> {
  const cfg = readOssEnvConfig();
  if (!cfg) throw new Error("OSS not configured");
  await getOssClient().send(
    new PutObjectCommand({
      Bucket: cfg.bucket,
      Key: key,
      Body: body,
      ContentType: contentType,
    }),
  );
}
