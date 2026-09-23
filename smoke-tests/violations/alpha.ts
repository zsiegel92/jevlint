export function double(value: number): number {
  const first = value * 2; // JEVLINT_SMOKE_BAD
  return first + 1; // JEVLINT_SMOKE_BAD
}
