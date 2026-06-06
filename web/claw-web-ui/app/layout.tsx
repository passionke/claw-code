import type { Metadata } from "next";
import "@copilotkit/react-ui/styles.css";
import "./globals.css";
import "./claw-copilot.css";

export const metadata: Metadata = {
  title: "Claw Web",
  description: "Claw Web — workspace + agent dock",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="zh-CN">
      <body>{children}</body>
    </html>
  );
}
