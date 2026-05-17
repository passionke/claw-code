import { SessionDiagnosticsView } from "@/components/SessionDiagnosticsView";
import "../../session-diagnostics.css";

type Props = {
  params: Promise<{ sessionId: string }>;
  searchParams: Promise<{ dsId?: string }>;
};

/** Standalone session diagnostics (not embedded in workspace). Author: kejiqing */
export default async function SessionDiagnosticsPage({ params, searchParams }: Props) {
  const { sessionId } = await params;
  const { dsId } = await searchParams;
  return <SessionDiagnosticsView sessionId={sessionId} dsIdParam={dsId ?? null} />;
}
