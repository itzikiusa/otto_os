const std = @import("std");
const suites = @import("suites");
const build_options = @import("build_options");

test "extra passes" {
    try std.testing.expectEqual(@as(i32, 5), suites.add(2, 3));
}

test "extra fails" {
    const got = suites.add(2, 3);
    try std.testing.expectEqual(@as(i32, 6), got);
}

test "extra panics" {
    if (build_options.panic) @panic("extra panic");
}
