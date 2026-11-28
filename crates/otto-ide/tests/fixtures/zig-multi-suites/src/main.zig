const std = @import("std");
const suites = @import("suites");

pub fn main() void {
    _ = suites.add(1, 2);
}

test "main adds" {
    try std.testing.expectEqual(@as(i32, 3), suites.add(1, 2));
}
