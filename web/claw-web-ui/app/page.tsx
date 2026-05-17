import { ClawCopilotProvider } from "@/components/ClawCopilotProvider";
import { ClawHome } from "@/components/ClawHome";
function publicUrl(envKey: string, fallback: string): string {
  const v = process.env[envKey];
  return v && v.length > 0 ? v : fallback;
}

export default function Page() {
  // Same-origin health BFF (browser fetch to :8090/:8088 often fails under Playwright/127.0.0.1).
  const bridgeUrl = "/api/health/bridge";
  const gatewayUrl = "/api/health/gateway";
  const tapUrl = publicUrl("NEXT_PUBLIC_CLAW_TAP_URL", "http://127.0.0.1:3000");
  const codeServerPort = process.env.NEXT_PUBLIC_CLAW_CODE_SERVER_PORT ?? "4101";
  const codeServerUrl =
    process.env.NEXT_PUBLIC_CLAW_CODE_SERVER_ENABLED === "1"
      ? `http://127.0.0.1:${codeServerPort}`
      : undefined;

  return (
    <ClawCopilotProvider>
      <ClawHome
        bridgeUrl={bridgeUrl}
        gatewayUrl={gatewayUrl}
        tapUrl={tapUrl}
        codeServerUrl={codeServerUrl}
      />
    </ClawCopilotProvider>
  );
}
