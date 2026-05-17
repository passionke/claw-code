import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  reactStrictMode: true,
  // Playwright / curl hit 127.0.0.1 while dev binds localhost (Next 15 dev cross-origin).
  allowedDevOrigins: ["127.0.0.1"],
};

export default nextConfig;
