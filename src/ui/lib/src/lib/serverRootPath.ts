let rootPathGetter = () => "/";

export const getServerRootPath = (): string => rootPathGetter();

export const setServerRootPath = (rootPath: string | (() => string)): void => {
  rootPathGetter = typeof rootPath === "string" ? () => rootPath : rootPath;
};
