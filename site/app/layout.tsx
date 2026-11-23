import type { Metadata } from "next";
import localFont from "next/font/local";
import { CS } from "../components/cowchat-site/copy";
import { THEME_SCRIPT } from "../components/theme";
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
  title: CS.meta.title,
  description: CS.meta.description,
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html
      lang="en"
      className={`${seasonSans.variable} ${seasonMix.variable}`}
      suppressHydrationWarning
    >
      <head>
        {/* Runs before paint so there is no flash of the wrong theme. Gallop's
            dark tokens activate via the .dark class on <html>. */}
        <script dangerouslySetInnerHTML={{ __html: THEME_SCRIPT }} />
      </head>
      <body>{children}</body>
    </html>
  );
}
