export function check(a: boolean, b: boolean): number {
  if (a) {
    if (b) {
      return 2;
    }
    return 1;
  }
  return 0;
}
