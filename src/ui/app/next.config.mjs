const development = process.env.NODE_ENV === "development";
const config = {
  ...(development
    ? {
        skipTrailingSlashRedirect: true,
        rewrites: async () =>
          ["auth", "lens", "v1", "models", "model_group", "health"].map((prefix) => ({
            source: `/${prefix}/:path*`,
            destination: `${process.env.LENS_DEV_API_URL ?? "http://127.0.0.1:4318"}/${prefix}/:path*`,
            basePath: false,
          })),
      }
    : { output: "export" }),
  basePath: "/ui",
  trailingSlash: true,
  images: { unoptimized: true },
  transpilePackages: ["@litellm/lens-ui"],
};
export default config;
