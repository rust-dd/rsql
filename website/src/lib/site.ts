export const REPOSITORY_URL = "https://github.com/rust-dd/rust-sql";
export const DOWNLOAD_URL = `${REPOSITORY_URL}/releases/latest`;
export const DOCS_URL = `${REPOSITORY_URL}#run-your-first-query`;
export const THEME_KEY = "rsql-website-theme";

export function downloadLabel() {
  const platform = navigator.userAgent.toLowerCase();
  if (/iphone|ipad|android/.test(platform)) return "Download RSQL";
  if (platform.includes("mac")) return "Download for macOS";
  if (platform.includes("win")) return "Download for Windows";
  if (platform.includes("linux")) return "Download for Linux";
  return "Download RSQL";
}
