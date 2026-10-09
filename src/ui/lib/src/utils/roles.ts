export const isProxyAdminRole = (role: string): boolean => {
  return role === "proxy_admin" || role === "Admin";
};

export const proxyAdminTierRoles = ["Admin", "Admin Viewer", "proxy_admin", "proxy_admin_viewer"];
export const isProxyAdminTierRole = (role: string): boolean => proxyAdminTierRoles.includes(role);
