import { useState } from 'preact/hooks';
import { readViewState, writeViewState } from './viewState.ts';

/** 同 `useState`,但值写穿到 `viewState` 存储,组件卸载重挂后恢复。 */
export function useViewState<T>(key: string, init: T): [T, (next: T | ((prev: T) => T)) => void] {
  const [value, setValue] = useState<T>(() => readViewState(key, init));
  const set = (next: T | ((prev: T) => T)) => {
    setValue((prev) => {
      const resolved = typeof next === 'function' ? (next as (p: T) => T)(prev) : next;
      writeViewState(key, resolved);
      return resolved;
    });
  };
  return [value, set];
}
