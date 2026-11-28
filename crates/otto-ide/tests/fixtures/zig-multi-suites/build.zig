const std = @import("std");

// Three test suites (lib, exe, extra), all wired into `zig build test`.
pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    const options = b.addOptions();
    options.addOption(bool, "panic", b.option(bool, "panic", "Add a test that panics") orelse false);

    const mod = b.addModule("suites", .{
        .root_source_file = b.path("src/root.zig"),
        .target = target,
    });
    const exe = b.addExecutable(.{
        .name = "suites",
        .root_module = b.createModule(.{
            .root_source_file = b.path("src/main.zig"),
            .target = target,
            .optimize = optimize,
            .imports = &.{.{ .name = "suites", .module = mod }},
        }),
    });
    b.installArtifact(exe);
    const extra = b.createModule(.{
        .root_source_file = b.path("src/extra.zig"),
        .target = target,
        .optimize = optimize,
        .imports = &.{.{ .name = "suites", .module = mod }},
    });
    extra.addOptions("build_options", options);

    const test_step = b.step("test", "Run tests");
    for ([_]*std.Build.Module{ mod, exe.root_module, extra }) |m| {
        const t = b.addTest(.{ .root_module = m });
        test_step.dependOn(&b.addRunArtifact(t).step);
    }
}
