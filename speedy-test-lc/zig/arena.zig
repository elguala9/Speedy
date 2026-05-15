const std = @import("std");
const Allocator = std.mem.Allocator;

/// A bump-pointer arena allocator.
/// All memory is freed at once when `deinit()` is called.
pub const Arena = struct {
    backing: Allocator,
    chunks: std.ArrayList([]u8),
    current: []u8 = &[_]u8{},
    offset: usize = 0,

    const CHUNK_SIZE: usize = 64 * 1024; // 64 KiB

    pub fn init(backing: Allocator) Arena {
        return .{
            .backing = backing,
            .chunks  = std.ArrayList([]u8).init(backing),
        };
    }

    pub fn deinit(self: *Arena) void {
        for (self.chunks.items) |chunk| self.backing.free(chunk);
        self.chunks.deinit();
    }

    pub fn allocator(self: *Arena) Allocator {
        return .{
            .ptr  = self,
            .vtable = &.{
                .alloc   = alloc,
                .resize  = resize,
                .free    = free,
            },
        };
    }

    fn alloc(ctx: *anyopaque, n: usize, log2_align: u8, _: usize) ?[*]u8 {
        const self: *Arena = @ptrCast(@alignCast(ctx));
        const align_mask = (@as(usize, 1) << @intCast(log2_align)) - 1;
        const aligned = (self.offset + align_mask) & ~align_mask;

        if (aligned + n > self.current.len) {
            const size = @max(CHUNK_SIZE, n + 64);
            const chunk = self.backing.alloc(u8, size) catch return null;
            self.chunks.append(chunk) catch {
                self.backing.free(chunk);
                return null;
            };
            self.current = chunk;
            self.offset  = 0;
        }

        const ptr = self.current.ptr + self.offset;
        self.offset += n;
        return ptr;
    }

    fn resize(_: *anyopaque, _: []u8, _: u8, _: usize, _: usize) bool { return false; }
    fn free(_: *anyopaque, _: []u8, _: u8, _: usize) void {}
};
