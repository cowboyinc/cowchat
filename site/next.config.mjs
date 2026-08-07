/** @type {import('next').NextConfig} */
const nextConfig = {
  // The site is 100% static (all routes prerender), so export to `out/` with
  // real index.html files. Serving raw `.next` as static assets 404s at `/`.
  output: "export",
  images: { unoptimized: true },
};

export default nextConfig;
