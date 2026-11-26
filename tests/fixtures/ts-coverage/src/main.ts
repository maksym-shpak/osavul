export function main(a: number, b: number): number {
  let r = 0;
  if (a > b) {
    r = a;
  } else {
    r = b;
  }
  switch (r) {
    case 1:
      r += 1;
      break;
    default:
      r += 2;
  }
  return r;
}
