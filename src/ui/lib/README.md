# Lens UI

`@litellm/lens-ui` owns the existing Lens workspace, trace viewer, investigations, findings, datasets, setup screens, shared components and assets. `src/ui/app` consumes this package for the standalone UI. LiteLLM's integration will consume the same package

The package exports TypeScript source and static image imports for a Next.js host. Add `@litellm/lens-ui` to `transpilePackages`, import `@litellm/lens-ui/styles.css` in the standalone layout, and provide TanStack Query, nuqs's Next app adapter, HotkeysProvider and ThemeProvider. The exported CSS includes its Tailwind source declaration so a tarball consumer can discover the package's classes. An embedded dashboard that already owns the same design tokens can scan the installed package alongside its existing stylesheet

The package has no imports into a host's `@/` alias or filesystem. React, Next, TanStack Query, nuqs, hotkeys and theme providers are peers so the host and Lens share those contexts

Initialize `configureLensHttp` before mounting Lens. Its getters preserve the host's API base and custom authorization header. Pass the current bearer token to `LensWorkspace` for embedded use; an empty token uses same-origin cookie authentication in standalone mode. The standalone shell exchanges its setup token through `/auth/session` and keeps it out of browser storage. The API's ClickHouse session persistence is still being completed

```tsx
configureLensHttp({
  getBaseUrl: getProxyBaseUrl,
  getAuthToken: () => currentAccessToken,
  getAuthHeaderName: getGlobalLitellmHeaderName,
  getServerRootPath: () => serverRootPath,
});

<LensHostProvider host={{ spendLogs: { lookup: uiSpendLogsCall, Drawer: LogDetailsDrawer } }}>
  <LensWorkspace accessToken={accessToken} userRole={userRole} readOnly={readOnly} />
</LensHostProvider>
```

The host supplies its native spend-log lookup and drawer through `LensHostProvider`. Lens retains the exact request ID, a lookup window padded by 30 minutes, and the return-to-trace action. Without a connected host, the spend-log link explains that LiteLLM must be connected; trace costs and captured request content remain visible

This is a development package, not a published release or a completed LiteLLM integration. Live standalone flows, the real embedded drawer, auth/permission parity and complete browser parity still require qualification
