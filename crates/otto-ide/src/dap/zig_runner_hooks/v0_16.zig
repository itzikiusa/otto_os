    const otto_no_exec = if (graph.environ_map.get("OTTO_ZIG_TEST_NO_EXEC")) |v| mem.eql(u8, v, "1") else false;
    const otto_filter = graph.environ_map.get("OTTO_ZIG_TEST_FILTER");
    const otto_file = graph.environ_map.get("OTTO_ZIG_TEST_FILE");
    if (otto_no_exec or (otto_filter != null and otto_filter.?.len > 0)) {
        const OttoTestEntry = struct {
            compile: *Step.Compile,
            run_step: ?*Step.Run,
        };
        const OttoWalker = struct {
            seen: std.AutoHashMap(*Step, void),
            entries: *std.array_list.Managed(OttoTestEntry),
            fn findCompileIn(s: *Step, depth: usize) ?*Step.Compile {
                if (depth > 8) return null;
                if (s.cast(Step.Compile)) |c| {
                    if (c.kind == .@"test") return c;
                }
                for (s.dependencies.items) |d| {
                    if (findCompileIn(d, depth + 1)) |found| return found;
                }
                return null;
            }
            fn visit(self: *@This(), s: *Step) !void {
                if (self.seen.contains(s)) return;
                try self.seen.put(s, {});
                if (s.cast(Step.Run)) |r| {
                    if (findCompileIn(s, 0)) |c| {
                        try self.entries.append(.{ .compile = c, .run_step = r });
                        return;
                    }
                }
                if (s.cast(Step.Compile)) |c| {
                    if (c.kind == .@"test") {
                        try self.entries.append(.{ .compile = c, .run_step = null });
                    }
                }
                for (s.dependencies.items) |dep| {
                    try self.visit(dep);
                }
            }
        };
        var otto_entries = std.array_list.Managed(OttoTestEntry).init(arena);
        var walker = OttoWalker{
            .seen = std.AutoHashMap(*Step, void).init(arena),
            .entries = &otto_entries,
        };
        if (targets.items.len > 0) {
            for (targets.items) |tname| {
                if (builder.top_level_steps.get(tname)) |tls| {
                    try walker.visit(&tls.step);
                }
            }
        } else if (builder.top_level_steps.get("test")) |tls| {
            try walker.visit(&tls.step);
        }
        var file_matched = false;
        if (otto_file) |want_file| {
            if (want_file.len > 0 and otto_entries.items.len > 1) {
                var matched = std.array_list.Managed(OttoTestEntry).init(arena);
                for (otto_entries.items) |e| {
                    if (e.compile.root_module.root_source_file) |lp| {
                        const rp = lp.getPath(builder);
                        const eq = mem.eql(u8, rp, want_file);
                        const want_suf = want_file.len > rp.len and want_file[want_file.len - rp.len - 1] == '/' and mem.endsWith(u8, want_file, rp);
                        const rp_suf = rp.len > want_file.len and rp[rp.len - want_file.len - 1] == '/' and mem.endsWith(u8, rp, want_file);
                        if (eq or want_suf or rp_suf) {
                            try matched.append(e);
                        }
                    }
                }
                if (matched.items.len > 0) {
                    otto_entries = matched;
                    file_matched = true;
                }
            }
        }
        if (otto_filter) |f| {
            if (f.len > 0) {
                const slice = try arena.dupe([]const u8, &.{f});
                for (otto_entries.items) |e| {
                    e.compile.filters = slice;
                }
            }
        }
        if (otto_no_exec) {
            if (otto_entries.items.len == 0) {
                fatal("otto: no Zig test compile step found in build.zig", .{});
            }
            const OttoEmitBinStep = struct {
                step: Step,
                entries: []const OttoTestEntry,
                fn make(step: *Step, _: Step.MakeOptions) anyerror!void {
                    const self: *@This() = @fieldParentPtr("step", step);
                    var otto_buf: [4096]u8 = undefined;
                    var otto_w = std.Io.File.stdout().writerStreaming(step.owner.graph.io, &otto_buf);
                    const out = &otto_w.interface;
                    for (self.entries) |e| {
                        const bp = e.compile.getEmittedBin().getPath(step.owner);
                        const src: []const u8 = if (e.compile.root_module.root_source_file) |lp| lp.getPath(step.owner) else "";
                        try out.print("OTTO_ZIG_TEST_BIN:{s}|{s}|{s}\n", .{ e.compile.name, src, bp });
                    }
                    try out.flush();
                }
            };
            const emit_step = try arena.create(OttoEmitBinStep);
            emit_step.* = .{
                .step = Step.init(.{
                    .id = .custom,
                    .name = "__otto_emit_test_bins",
                    .owner = builder,
                    .makeFn = OttoEmitBinStep.make,
                }),
                .entries = otto_entries.items,
            };
            for (otto_entries.items) |e| {
                // Request the binary while configuring: a test step that only compiles
                // (no run or install step) never asks for it, and getPath would panic.
                _ = e.compile.getEmittedBin();
                emit_step.step.dependOn(&e.compile.step);
            }
            const compile_only = builder.step("__otto_compile_tests", "Compile Zig tests without running");
            compile_only.dependOn(&emit_step.step);
            targets.clearRetainingCapacity();
            try targets.append("__otto_compile_tests");
        } else if (file_matched) {
            const filtered_run = builder.step("__otto_run_filtered_tests", "Run only the selected Zig test suite");
            var added_any = false;
            for (otto_entries.items) |e| {
                if (e.run_step) |rs| {
                    filtered_run.dependOn(&rs.step);
                    added_any = true;
                }
            }
            if (added_any) {
                targets.clearRetainingCapacity();
                try targets.append("__otto_run_filtered_tests");
            }
        }
    }
