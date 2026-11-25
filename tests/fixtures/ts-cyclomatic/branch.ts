export function classify(value: number): string {
  const label = "value";
  if (value < 0) {
    return "negative";
  }
  return label + value;
}
