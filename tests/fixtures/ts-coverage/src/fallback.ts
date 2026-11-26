"use strict";
const a = 1; let b = 2;
if (a) { b = 3; }
function f(): number {
  "use asm";
  for (let i = 0; i < 2; i++) b += i;
  return (
    b
  );
}
type T = number;
interface I { x: number }
f();
