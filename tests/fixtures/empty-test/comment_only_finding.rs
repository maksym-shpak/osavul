fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn check_sum() {
    assert_eq!(add(2, 3), 5);
}

#[test]
fn sum_is_checked() {
    // перевірку ще не написано
}
