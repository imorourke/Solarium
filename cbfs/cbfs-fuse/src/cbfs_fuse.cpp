#include <array>
#include <cassert>
#include <cerrno>
#include <cstddef>
#include <cstdio>
#include <cstdlib>
#include <ctime>
#include <errno.h>
#include <fcntl.h>
#include <functional>
#include <fuse3/fuse.h>
#include <fuse3/fuse_opt.h>
#include <iostream>
#include <limits>
#include <memory>
#include <mutex>
#include <shared_mutex>
#include <sys/stat.h>
#include <unordered_map>

#include "cbfs_fuse.hpp"
#include "cbfs/src/main.rs.h"

#ifdef WIN32
#define fuse_file_info_t fuse_file_info
#define fuse_filler_t fuse_fill_dir_t
#define fuse_statfs_t fuse_statvfs
#else
#ifdef __APPLE__
#define fuse_stat fuse_darwin_attr
#define fuse_off_t off_t
#define fuse_mode_t mode_t
#define fuse_statvfs statvfs
#define fuse_file_info_t file_info
#define fuse_timespec timespec
#define fuse_filler_t fuse_darwin_fill_dir_t
#define fuse_statfs_t statfs
#else
#define fuse_stat stat
#define fuse_off_t off_t
#define fuse_mode_t mode_t
#define fuse_statvfs statvfs
#define fuse_file_info_t file_info
#define fuse_timespec timespec
#define fuse_filler_t fuse_fill_dir_t
#define fuse_statfs_t fuse_statvfs
#endif
#endif

struct cbfs_error {
    CbFsError result;

    cbfs_error(CbFsError err) : result{err} {}

    int get_return_code() const {
        switch (result) {
        case CbFsError::EntryNotDirectory:
            return -ENOTDIR;
        case CbFsError::EntryNotFile:
        case CbFsError::EntryNotFound:
        case CbFsError::InvalidEntry:
            return -ENOENT;
        case CbFsError::NoSpace:
            return -ENOSPC;
        case CbFsError::DuplicateName:
            return -EEXIST;
        case CbFsError::InvalidName:
            return -ENAMETOOLONG;
        default:
            return -ENOSYS;
        }
    }
};

struct CbFuseState {
    rust::Box<CbFs> fs;
    uint16_t block_size{};
    mutable std::shared_mutex lock{};
    std::string base_file{};
    bool read_only{ false };
    std::unordered_map<uint16_t, fuse_mode_t> current_modes{};

    CbFuseState(const std::string& base_name, bool randomize)
        : fs{ CbFs::open(base_name, randomize) } {
        // Empty Constructor
    }

    bool save_fs() {
        if (!read_only) {
            try {
                fs->save(base_file);
                return true;
            } catch (const rust::Error&) {
                return false;
            }
        } else {
            return true;
        }
    }

    CbFsEntry get_entry(const char* path) const { return fs->entry_by_path(path); }

    CbFsEntry get_entry(uint16_t id) const { return fs->entry_by_id(id); }

    static CbFuseState* get_instance() { return static_cast<CbFuseState*>(fuse_get_context()->private_data); }

    ~CbFuseState() { save_fs(); }
};

static void* cbfs_fuse_init(struct fuse_conn_info*, struct fuse_config* config) {
    CbFuseState* state = static_cast<CbFuseState*>(fuse_get_context()->private_data);
    std::unique_lock lk(state->lock);

    config->use_ino = false;
    config->kernel_cache = true;
    state->block_size = state->fs->get_stats().block_size;

    return state;
}

static void cbfs_fuse_destroy(void* private_data) {
    auto state = static_cast<CbFuseState*>(private_data);
    if (state != nullptr) {
        delete state;
    }
}

static fuse_timespec millis_to_timespec(int64_t millis) {
    return fuse_timespec{
        .tv_sec = static_cast<int32_t>(millis / 1000),
        .tv_nsec = static_cast<int32_t>((millis % 1000) * 1000000),
    };
}

static int64_t timespec_to_millis(const fuse_timespec& ts) { return ts.tv_sec * 1000 + ts.tv_nsec / 1000000; }

static struct fuse_stat cbfs_util_getstat(const CbFuseState& state, const CbFsEntry& entry) {
    struct fuse_context* ctx = fuse_get_context();
    struct fuse_stat stbuf{};

    fuse_mode_t mode_val{};

#ifdef WIN32
    constexpr fuse_mode_t FILE_MODE = S_IFREG | 0777;
    constexpr fuse_mode_t FILE_EXEC_MODE = S_IFREG | 0777;
    constexpr fuse_mode_t FOLDER_MODE = S_IFDIR | 0777;
#else
    constexpr fuse_mode_t FILE_MODE = S_IFREG | 0644;
    constexpr fuse_mode_t FILE_EXEC_MODE = S_IFREG | 0755;
    constexpr fuse_mode_t FOLDER_MODE = S_IFDIR | 0755;
#endif

    if (entry.entry_type == CbFsEntryType::Directory) {
        mode_val = FOLDER_MODE;
    } else if (entry.entry_type == CbFsEntryType::File) {
        if (state.fs->is_executable(entry.entry_id)) {
            mode_val = FILE_EXEC_MODE;
        } else {
            mode_val = FILE_MODE;
        }
    }

    constexpr uint32_t DEFAULT_BLOCK_SIZE = 512;
    const uint32_t block_count = entry.size_bytes / DEFAULT_BLOCK_SIZE + ((entry.size_bytes == DEFAULT_BLOCK_SIZE) ? 0 : 1);
    const auto time_val = millis_to_timespec(entry.last_time.to_millis());

#ifdef __APPLE__
#define STATFIELD(NAME) NAME
#define TIMEFIELD(NAME) NAME##espec
#else
#define STATFIELD(NAME) st_##NAME
#define TIMEFIELD(NAME) STATFIELD(NAME)
#endif

    stbuf.STATFIELD(ino) = entry.entry_id;
    stbuf.STATFIELD(blksize) = state.block_size;
    stbuf.STATFIELD(blocks) = block_count;
    stbuf.STATFIELD(size) = entry.size_bytes;
    stbuf.STATFIELD(uid) = ctx->uid;
    stbuf.STATFIELD(gid) = ctx->gid;
    stbuf.STATFIELD(nlink) = 1;
    stbuf.STATFIELD(mode) = mode_val;
    stbuf.TIMEFIELD(atim) = time_val;
    stbuf.TIMEFIELD(mtim) = time_val;
    stbuf.TIMEFIELD(ctim) = time_val;

#undef STATFIELD
#undef TIMEFIELD

    return stbuf;
}

static struct fuse_stat util_getstat(const CbFuseState& state, const char* path) { return cbfs_util_getstat(state, state.get_entry(path)); }

static struct fuse_stat util_getstat(const CbFuseState& state, uint16_t entry_val) {
    return cbfs_util_getstat(state, state.get_entry(entry_val));
}

static int cbfs_check_err_int(std::function<int()> func) {
    try {
        return func();
    } catch (const rust::Error& err) {
        return cbfs_error(cbfs_error_from_str(err.what())).get_return_code();
    } catch (const cbfs_error& err) {
        return err.get_return_code();
    }
}

static int cbfs_check_err(std::function<void()> func) {
    return cbfs_check_err_int([&]() {
        func();
        return 0;
    });
}

static int cbfs_fuse_getattr(const char* path, struct fuse_stat* stbuf, struct fuse_file_info*) {
    const auto state = CbFuseState::get_instance();
    std::shared_lock lk(state->lock);

    return cbfs_check_err([&]() { *stbuf = util_getstat(*state, path); });
}

static int cbfs_fuse_open(const char* path, struct fuse_file_info* fi) {
    const auto state = CbFuseState::get_instance();
    std::shared_lock lk(state->lock);

    return cbfs_check_err([&]() { fi->fh = state->get_entry(path).entry_id; });
}

static int cbfs_fuse_opendir(const char* path, struct fuse_file_info* fi) {
    const auto state = CbFuseState::get_instance();
    std::shared_lock lk(state->lock);

    return cbfs_check_err([&]() {
        CbFsEntry entry = state->get_entry(path);

        fi->fh = entry.entry_id;
#ifndef WIN32
        fi->cache_readdir = true;
#endif
    });
}

static int cbfs_fuse_read(const char*, char* buf, size_t size, fuse_off_t offset, struct fuse_file_info* fi) {
    const auto state = CbFuseState::get_instance();
    std::shared_lock lk(state->lock);

    return cbfs_check_err_int([&]() {
        if (static_cast<int32_t>(size) > std::numeric_limits<int32_t>::max()) {
            return -ENOSYS;
        }

        return static_cast<int32_t>(state->fs->read_entry_data(
            static_cast<uint16_t>(fi->fh), static_cast<uint32_t>(offset), rust::Slice<uint8_t>(reinterpret_cast<uint8_t*>(buf), size)
        ));
    });
}

static int
cbfs_fuse_readdir(const char*, void* buf, fuse_filler_t filler, fuse_off_t offset, struct fuse_file_info* fi, fuse_readdir_flags) {
    const auto state = CbFuseState::get_instance();
    std::shared_lock lk(state->lock);

    return cbfs_check_err([&]() {
        const CbFsEntry entry = state->get_entry(static_cast<uint16_t>(fi->fh));

        if (entry.entry_type != CbFsEntryType::Directory) {
            throw cbfs_error(CbFsError::EntryNotDirectory); // Replaced ENOENT with ENODIR
        }

        const auto add_entry = [&buf, &filler](const struct fuse_stat* val, std::string name) -> void {
            if (0 != filler(buf, name.c_str(), val, 0, FUSE_FILL_DIR_PLUS)) {
                throw cbfs_error(CbFsError::UnknownError);
            }
        };

        const auto entries = state->fs->read_dir(entry.entry_id);
        const auto entry_size = entries.size();

        for (size_t i = static_cast<size_t>(offset); i < entry_size + 2; ++i) {
            if (i == 0) {
                const auto val = util_getstat(*state, entry.entry_id);
                add_entry(&val, ".");
            } else if (i == 1) {
                const uint16_t parent_id = state->fs->get_parent_node(entry.entry_id);
                if (parent_id != 0) {
                    const auto val = util_getstat(*state, parent_id);
                    add_entry(&val, "..");
                } else {
                    add_entry(nullptr, "..");
                }
            } else {
                const CbFsEntry& local_entry = entries[i - 2];
                const auto val = util_getstat(*state, local_entry.entry_id);
                add_entry(&val, std::string(local_entry.name)); // TODO - This may not work with pointer shenanigans
            }
        }
    });
}

static int cbfs_fuse_write(const char* path, const char* data, size_t size, fuse_off_t offset, struct fuse_file_info* fi) {
    const auto state = CbFuseState::get_instance();
    std::unique_lock lk(state->lock);

    return cbfs_check_err_int([&]() {
        uint16_t entry_val;
        CbFsEntry entry{};
        if (fi != nullptr && fi->fh != 0) {
            entry_val = static_cast<uint16_t>(fi->fh);
            entry = state->get_entry(entry_val);
        } else {
            entry = state->get_entry(path);
            entry_val = entry.entry_id;
        }

        const uint32_t want_size = static_cast<uint32_t>(size + offset);

        if (want_size > entry.size_bytes) {
            state->fs->truncate(entry_val, want_size);
        }

        return static_cast<int>(state->fs->write_entry_data(
            entry_val, static_cast<uint32_t>(offset), rust::Slice<const uint8_t>(reinterpret_cast<const uint8_t*>(data), size)
        ));
    });
}

static int cbfs_fuse_truncate(const char*, fuse_off_t size, fuse_file_info* fi) {
    const auto state = CbFuseState::get_instance();
    std::unique_lock lk(state->lock);

    return cbfs_check_err([&]() { state->fs->truncate(static_cast<uint16_t>(fi->fh), static_cast<uint32_t>(size)); });
}

static int cbfs_fuse_create(const char* path, fuse_mode_t mode, struct fuse_file_info* fi) {
    const auto state = CbFuseState::get_instance();
    std::unique_lock lk(state->lock);

    const bool can_truncate = (mode & (O_CREAT | O_TRUNC)) == (O_CREAT | O_TRUNC);

    return cbfs_check_err([&]() {
        fi->fh = state->fs->create_entry(path, CbFsEntryType::File, can_truncate);
    });
}

static int cbfs_fuse_rename(const char* path, const char* new_path, [[maybe_unused]] unsigned int flags) {
    const auto state = CbFuseState::get_instance();
    std::unique_lock lk(state->lock);

#ifdef RENAME_EXCHANGE
    if ((flags & RENAME_EXCHANGE) == RENAME_EXCHANGE) {
        return -ENOSYS;
    }
#endif

    return cbfs_check_err([&]() {
#ifdef RENAME_EXCHANGE
        const bool can_replace = (flags & RENAME_NOREPLACE) == 0;
#else
        const bool can_replace = false;
#endif
        state->fs->rename_entry(path, new_path, can_replace);
    });
}

static int cbfs_fuse_unlink(const char* path) {
    const auto state = CbFuseState::get_instance();
    std::unique_lock lk(state->lock);

    return cbfs_check_err([&]() { state->fs->remove_entry(path, CbFsEntryType::File); });
}

static int cbfs_fuse_mkdir(const char* path, fuse_mode_t) {
    const auto state = CbFuseState::get_instance();
    std::unique_lock lk(state->lock);

    return cbfs_check_err([&]() { state->fs->create_entry(path, CbFsEntryType::Directory, false); });
}

static int cbfs_fuse_mknod(const char* path, fuse_mode_t mode, dev_t) {
    struct fuse_file_info fi{};
    return cbfs_fuse_create(path, mode, &fi);
}

static int cbfs_fuse_rmdir(const char* path) {
    const auto state = CbFuseState::get_instance();
    std::unique_lock lk(state->lock);

    return cbfs_check_err([&]() { state->fs->remove_entry(path, CbFsEntryType::Directory); });
}

static int cbfs_fuse_statfs(const char*, struct fuse_statfs_t* statfs) {
    const auto state = CbFuseState::get_instance();
    std::shared_lock lk(state->lock);

    return cbfs_check_err([&]() {
        const CbFsStats fs_stats = state->fs->get_stats();

#ifndef __APPLE__
        statfs->f_namemax = FILENAME_MAX;
        statfs->f_frsize = fs_stats.block_size;
#endif
        statfs->f_bsize = fs_stats.block_size;

        statfs->f_blocks = fs_stats.num_blocks;
        statfs->f_bfree = fs_stats.free_blocks;
        statfs->f_bavail = fs_stats.free_blocks;

        statfs->f_files = fs_stats.num_blocks;
        statfs->f_ffree = fs_stats.free_blocks;
#ifndef __APPLE__
        statfs->f_favail = fs_stats.free_blocks;
#endif

#ifndef __APPLE__
        statfs->f_fsid = 0xA80E83BC;
        statfs->f_flag = 0;
#endif
    });
}

static int cbfs_fuse_fsync(const char*, int, struct fuse_file_info*) {
    const auto state = CbFuseState::get_instance();
    std::shared_lock lk(state->lock);

    if (!state->save_fs()) {
        return -ENOSYS;
    } else {
        return 0;
    }
}

static int cbfs_fuse_chmod(const char* path, fuse_mode_t file_mode, struct fuse_file_info* fi) {
    const auto state = CbFuseState::get_instance();
    std::unique_lock lk(state->lock);

    return cbfs_check_err([&]() {
        CbFsEntry entry{};
        if (fi != nullptr) {
            entry = state->get_entry(static_cast<uint16_t>(fi->fh));
        } else {
            entry = state->get_entry(path);
        }

        if (entry.entry_type == CbFsEntryType::File) {
            state->fs->set_executable(entry.entry_id, (file_mode & 0x0100) != 0);
        }
    });
}

static int cbfs_fuse_utimens(const char* path, const fuse_timespec* tv, fuse_file_info* fi) {
    const auto state = CbFuseState::get_instance();
    std::unique_lock lk(state->lock);

    return cbfs_check_err([&]() {
        uint16_t hdl{};
        if (fi == nullptr) {
            hdl = state->get_entry(path).entry_id;
        } else {
            hdl = static_cast<uint16_t>(fi->fh);
        }

#ifdef WIN32
        constexpr int64_t UTIME_NOW = 0;
#endif

        if (tv != nullptr && tv->tv_nsec != UTIME_NOW) {
            const CbFsTime tvi = CbFsTime::from_millis(timespec_to_millis(*tv));
            state->fs->entry_set_time(hdl, tvi);
        } else {
            state->fs->entry_set_time_current(hdl);
        }
    });
}

struct options_t {
    bool file_read_only;
    bool randomize;
    const char* base_file;
};

enum {
    KEY_VERSION,
    KEY_HELP,
};

#define CBFS_OPTION(t, p) { t, offsetof(options_t, p), 1 }
#ifdef WIN32
#undef FUSE_OPT_KEY
#define FUSE_OPT_KEY(templ, key) { templ, static_cast<unsigned int>(-1), key }
#endif

static const auto cbfs_option_spec = std::to_array<struct fuse_opt>({
    CBFS_OPTION("file=%s", base_file),
    CBFS_OPTION("mem", file_read_only),
    CBFS_OPTION("rand", randomize),
    FUSE_OPT_KEY("--help", KEY_HELP),
    FUSE_OPT_KEY("-h", KEY_HELP),
    FUSE_OPT_END,
});

#undef CBFS_OPTION

static constexpr struct fuse_operations generate_fuse_opers() {
    struct fuse_operations opers{};
    opers.init = cbfs_fuse_init;
    opers.destroy = cbfs_fuse_destroy;
    opers.getattr = cbfs_fuse_getattr;
    opers.open = cbfs_fuse_open;
    opers.opendir = cbfs_fuse_opendir;
    opers.read = cbfs_fuse_read;
    opers.readdir = cbfs_fuse_readdir;
    opers.write = cbfs_fuse_write;
    opers.truncate = cbfs_fuse_truncate;
    opers.create = cbfs_fuse_create;
    opers.rename = cbfs_fuse_rename;
    opers.unlink = cbfs_fuse_unlink;
    opers.mkdir = cbfs_fuse_mkdir;
    opers.mknod = cbfs_fuse_mknod;
    opers.rmdir = cbfs_fuse_rmdir;
    opers.statfs = cbfs_fuse_statfs;
    opers.fsync = cbfs_fuse_fsync;
    opers.fsyncdir = cbfs_fuse_fsync;
    opers.chmod = cbfs_fuse_chmod;
    opers.utimens = cbfs_fuse_utimens;
    return opers;
}

static const struct fuse_operations cbfs_fuse_oper = generate_fuse_opers();

struct FuseArgContainer {
    struct fuse_args args{};

    FuseArgContainer(int argc, char* argv[]) { args = FUSE_ARGS_INIT(argc, argv); }

    FuseArgContainer(std::span<const char*> argv) {
        for (const auto& i : argv) {
            fuse_opt_add_arg(&args, i);
        }
    }

    FuseArgContainer(const FuseArgContainer&) = delete;
    FuseArgContainer(FuseArgContainer&&) = delete;
    FuseArgContainer& operator=(FuseArgContainer&) = delete;
    FuseArgContainer& operator=(FuseArgContainer&&) = delete;

    ~FuseArgContainer() { fuse_opt_free_args(&args); }
};

static void cbfs_print_usage(const char* prog);

static int cbfs_opt_proc(
    void*,       // data
    const char*, // arg
    int key,
    struct fuse_args* outargs
) {
    switch (key) {
    case KEY_HELP:
        cbfs_print_usage(outargs->argv[0]);
        exit(0);
    case KEY_VERSION:
        fprintf(stdout, "cbfs version 0.1\n");
        fuse_opt_add_arg(outargs, "--version");
        fuse_main(outargs->argc, outargs->argv, &cbfs_fuse_oper, nullptr);
        exit(0);
    }
    return 1;
}

static void cbfs_print_usage(const char* prog) {
    const auto version = std::string(cbfs_get_version());
    fprintf(
        stdout,
        "usage: %s [options] <device|image> <mountpoint>\n"
        "version: %s\n"
        "\n"
        "cbfs options:\n"
        "    -o mem                 operates only in memory\n"
        "    -o rand                randomizes used sectors\n",
        prog,
        version.c_str()
    );

    auto argvals = std::to_array<const char*>({
        "",
        "-h",
    });
    const auto args = FuseArgContainer(argvals);
    fuse_main(args.args.argc, args.args.argv, &cbfs_fuse_oper, nullptr);
}

int32_t cxx_fuse_main(rust::Slice<const rust::String> args_in) {
    // Extract out the device path
    std::vector<std::string> args_tmp;
    for (const auto& s : args_in) {
        args_tmp.push_back(std::string(s));
    }

    std::vector<char*> args_tmp2;
    for (const auto& s : args_tmp) {
        args_tmp2.push_back(const_cast<char*>(s.c_str()));
    }

    auto argv = args_tmp2.data();
    auto argc = args_tmp2.size();

    options_t options{};
    if ((argc < 3) || (argv[argc - 2][0] == '-') || (argv[argc - 1][0] == '-')) {
        cbfs_print_usage(argv[0]);
        return 1;
    } else {
        options.base_file = argv[argc - 2];
        argv[argc - 2] = argv[argc - 1];
        argc -= 1;
    }

    FuseArgContainer args(argc, argv);
    fuse_opt_parse(&args.args, &options, cbfs_option_spec.data(), cbfs_opt_proc);

    if (options.base_file == nullptr) {
        std::cerr << "Unable to open base file - please specify\n";
        return 1;
    }

#ifndef WIN32
    std::string base_file;
    {
        char* resolved_input = realpath(options.base_file, nullptr);
        if (resolved_input != nullptr) {
            base_file = resolved_input;
            free(resolved_input);
        } else {
            base_file = options.base_file;
        }
    }
#else
    const std::string base_file = options.base_file;
#endif

    std::unique_ptr<CbFuseState> state = std::make_unique<CbFuseState>(base_file, options.randomize);

    state->read_only = options.file_read_only != 0;

    return fuse_main(args.args.argc, args.args.argv, &cbfs_fuse_oper, state.release());
}
