/** Join the truthy class names. CSS Module lookups can be undefined under `noUncheckedIndexedAccess`. */
export function cx(...classes: (string | false | null | undefined)[]): string {
  return classes.filter(Boolean).join(' ');
}
