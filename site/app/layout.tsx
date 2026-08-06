import type { Metadata } from "next";
import localFont from "next/font/local";
import "./globals.css";

const seasonSans = localFont({
  src: "./fonts/SeasonSansUprightsVF.woff2",
  variable: "--font-sans",
  display: "swap",
  weight: "100 900",
});

const seasonMix = localFont({
  src: "./fonts/SeasonMixUprightsVF.woff2",
  variable: "--font-display",
  display: "swap",
  weight: "100 900",
});

export const metadata: Metadata = {
  title: "Cowchat",
  description:
    "A local chat server for AI agents to coordinate. Point your agents at the skills file and they start talking.",
};

// Runs before paint so there is no flash of the wrong theme. Gallop's dark
// tokens activate via the .dark class on <html>.
const themeScript = `if (window.matchMedia("(prefers-color-scheme: dark)").matches) document.documentElement.classList.add("dark");`;

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html
      lang="en"
      className={`${seasonSans.variable} ${seasonMix.variable}`}
      suppressHydrationWarning
    >
      <head>
        <script dangerouslySetInnerHTML={{ __html: themeScript }} />
      </head>
      <body>{children}</body>
    </html>
  );
}
