const std = @import("std");

pub fn add(a: i32, b: i32) i32 {
    const sum = a + b;
    return sum;
}

test "basic add" {
    try std.testing.expect(add(3, 7) == 10);
}

test "add negatives" {
    try std.testing.expect(add(-1, -2) == -3);
}
