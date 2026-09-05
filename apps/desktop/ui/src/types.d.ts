type EffectCleanup = void | (() => void);

declare module 'react' {
  export function useEffect(effect: () => EffectCleanup, dependencies?: readonly unknown[]): void;
  export function useMemo<T>(factory: () => T, dependencies: readonly unknown[]): T;
  export function useState<T>(initial: T): [T, (value: T | ((current: T) => T)) => void];
}
