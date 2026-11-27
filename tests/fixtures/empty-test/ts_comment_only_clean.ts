import { add } from "./math";

describe("math", () => {
  test("adds numbers", async function () {
    add(2, 3);
  });
});
