import { ClawCopilotProvider } from "@/components/ClawCopilotProvider";
import { ClawHome } from "@/components/ClawHome";
import { gatewayBaseUrl } from "@/lib/claw-config";

function publicUrl(envKey: string, fallback: string): string {
  const v = process.env[envKey];
  return v && v.length > 0 ? v : fallback;
}

export default function Page() {
  const bridgeUrl = publicUrl("NEXT_PUBLIC_CLAW_AGUI_BRIDGE_URL", "http://127.0.0.1:8090");
  const gatewayUrl = publicUrl("NEXT_PUBLIC_CLAW_GATEWAY_BASE_URL", gatewayBaseUrl());
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
