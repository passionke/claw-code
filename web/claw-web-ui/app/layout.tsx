import type { Metadata } from "next";
import "./globals.css";

export const metadata: Metadata = {
  title: "Claw Web",
  description: "CopilotKit sidebar for Claw AG-UI stack",
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en">
      <body>{children}</body>
    </html>
  );
}
