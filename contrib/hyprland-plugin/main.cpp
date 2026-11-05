// anima-hypr: shares workspace state with animad through shared memory.
//  - publishes each monitor's active workspace and the offset Hyprland draws
//    it at (its `m_renderOffset`, which swipes and workspace animations move)
//  - places animad's character windows (app_id "anima-character"): Wayland
//    clients can't position their own windows, so animad writes where each
//    one goes and the plugin moves it there before every frame, without
//    animation. Hyprland then slides them with their workspace like any
//    other window. Only windows of the animad process named in a record are
//    ever touched.
//
// Layout of $XDG_RUNTIME_DIR/anima-hyprland.shm, 20480 bytes (mirrored in
// src/backend/governor.rs):
//   0    u32  magic, 0 when the plugin is unloaded
//   4    u32  version
//   8    char instance[120]  HYPRLAND_INSTANCE_SIGNATURE
//   128  16 x 64-byte monitor records:
//        u32 seq (odd while being written), i32 active workspace id,
//        f32 offset in workspace steps (monitor width + gaps_workspaces),
//        u32 unused, char name[48]
//   2048 256 x 64-byte window records, written by animad (index = actor):
//        u32 seq (odd while being written), u32 pid, u32 actor,
//        i32 workspace id, f32 x, f32 y (logical, from the monitor's
//        top-left), char monitor[40]

#include <src/plugins/PluginAPI.hpp>
#include <src/event/EventBus.hpp>
#include <src/output/Monitor.hpp>
#include <src/workspace/HLWorkspace.hpp>
#include <src/config/ConfigValue.hpp>
#include <src/config/shared/actions/ConfigActions.hpp>
#include <src/desktop/state/FocusState.hpp>
#include <src/desktop/state/WindowState.hpp>
#include <src/desktop/view/window/Window.hpp>
#include <src/desktop/view/window/WindowBackend.hpp>
#include <src/desktop/view/window/WindowMetadata.hpp>
#include <src/managers/eventLoop/EventLoopManager.hpp>
#include <src/protocols/core/Compositor.hpp>
#include <src/state/workspace/State.hpp>
#include <src/state/MonitorState.hpp>

#include <atomic>
#include <cstdlib>
#include <cstring>
#include <string>

#include <fcntl.h>
#include <sys/mman.h>
#include <unistd.h>

namespace {
    constexpr uint32_t MAGIC        = 0x414e4859; // "ANHY"
    constexpr uint32_t VERSION      = 4;
    constexpr size_t   WINDOWS      = 2048;
    constexpr int      MAX_WINDOWS  = 256;
    constexpr size_t   SIZE         = 20480;
    constexpr int      MAX_MONITORS = 16;

    struct SMonitorRecord {
        std::atomic<uint32_t> seq;
        int32_t               active;
        float                 offset;
        uint32_t              unused;
        char                  name[48];
    };

    struct SShared {
        std::atomic<uint32_t> magic;
        uint32_t              version;
        char                  instance[120];
        SMonitorRecord        monitors[MAX_MONITORS];
    };

    struct SWindowRecord {
        std::atomic<uint32_t> seq;
        uint32_t              pid;
        uint32_t              actor;
        int32_t               workspace;
        float                 x, y;
        char                  monitor[40];
    };

    static_assert(sizeof(SMonitorRecord) == 64);
    static_assert(sizeof(SWindowRecord) == 64);
    static_assert(sizeof(SShared) <= WINDOWS);
    static_assert(WINDOWS + MAX_WINDOWS * sizeof(SWindowRecord) <= SIZE);

    struct SPlaced {
        uint32_t     pid = 0, actor = 0;
        PHLWINDOWREF window;
    };
    std::array<SPlaced, MAX_WINDOWS> g_placed;
    bool                             g_moveQueued = false;
    PHLWINDOWREF                     g_lastFocus;
    bool                             g_refocusQueued = false;

    bool isCharacter(const PHLWINDOW& w) {
        return w && w->metadata().appID() == "anima-character";
    }

    // Hyprland hit-tests floating windows by their box, not their input
    // region, so a screen-sized character window would take every hover and
    // click. Before a pointer move (or click) is handled, only character
    // windows with a character under the cursor stay focusable.
    void passThrough(const Vector2D& pos) {
        for (const auto& w : Desktop::windowState()->windows()) {
            if (!isCharacter(w) || !w->mapped())
                continue;
            const auto SURF   = w->resource();
            const auto LOCAL  = pos - w->positionAnimation()->value();
            const bool INSIDE = SURF && SURF->m_current.effectiveInputRegion().containsPoint(LOCAL);
            if (w->m_ruleApplicator->noFocus().valueOrDefault() == INSIDE)
                Config::Actions::setProp("no_focus", INSIDE ? "0" : "1", w);
        }
    }

    // Character windows have to take clicks (dragging, petting), but a click
    // must not leave keyboard focus on them: hand it back right away.
    void keepFocus() {
        const auto F = Desktop::focusState()->window();
        if (!isCharacter(F)) {
            g_lastFocus = F;
            return;
        }
        if (g_refocusQueued)
            return;
        g_refocusQueued = true;
        g_pEventLoopManager->doLater([] {
            g_refocusQueued = false;
            const auto F = Desktop::focusState()->window();
            if (!isCharacter(F))
                return;
            if (const auto LAST = g_lastFocus.lock(); validMapped(LAST) && !isCharacter(LAST))
                Desktop::focusState()->fullWindowFocus(LAST, Desktop::FOCUS_REASON_DESKTOP_STATE_CHANGE);
            else
                Desktop::focusState()->fullWindowFocus(nullptr, Desktop::FOCUS_REASON_DESKTOP_STATE_CHANGE);
        });
    }

    HANDLE              g_handle = nullptr;
    SShared*            g_shared = nullptr;
    CHyprSignalListener g_listener;
    CHyprSignalListener g_moveListener;

    SMonitorRecord*     slotFor(const std::string& name) {
        SMonitorRecord* empty = nullptr;
        for (auto& m : g_shared->monitors) {
            if (std::strncmp(m.name, name.c_str(), sizeof(m.name)) == 0)
                return &m;
            if (!empty && m.name[0] == '\0')
                empty = &m;
        }
        return empty;
    }

    void publish(const PHLMONITOR& mon) {
        if (!g_shared || !mon || mon->m_name.empty())
            return;

        static auto PGAP = CConfigValue<Config::INTEGER>("general:gaps_workspaces");

        int32_t     active = -1;
        float       offset = 0.F;
        if (const auto WS = mon->m_activeWorkspace) {
            if (const auto ID = WS->numberedID())
                active = sc<int32_t>(*ID);
            const double STEP = mon->m_size.x + *PGAP;
            if (STEP > 0)
                offset = sc<float>(WS->m_renderOffset->value().x / STEP);
        }

        auto* rec = slotFor(mon->m_name);
        if (!rec)
            return;
        if (rec->name[0] != '\0' && rec->active == active && rec->offset == offset)
            return;

        // seqlock: readers retry while seq is odd or changed under them
        const uint32_t SEQ = rec->seq.load(std::memory_order_relaxed);
        rec->seq.store(SEQ + 1, std::memory_order_relaxed);
        std::atomic_thread_fence(std::memory_order_release);
        rec->active = active;
        rec->offset = offset;
        std::strncpy(rec->name, mon->m_name.c_str(), sizeof(rec->name) - 1);
        rec->seq.store(SEQ + 2, std::memory_order_release);
    }

    SWindowRecord* windowRecords() {
        return rc<SWindowRecord*>(rc<char*>(g_shared) + WINDOWS);
    }

    // consistent copy of a record, false if it's unused or being written
    bool readRecord(int i, SWindowRecord& out, std::string& monitor) {
        auto&          rec = windowRecords()[i];
        const uint32_t S1  = rec.seq.load(std::memory_order_acquire);
        if (S1 & 1)
            return false;
        out.pid       = rec.pid;
        out.actor     = rec.actor;
        out.workspace = rec.workspace;
        out.x         = rec.x;
        out.y         = rec.y;
        char name[sizeof(rec.monitor)];
        std::memcpy(name, rec.monitor, sizeof(name));
        std::atomic_thread_fence(std::memory_order_acquire);
        if (rec.seq.load(std::memory_order_relaxed) != S1 || out.pid == 0)
            return false;
        name[sizeof(name) - 1] = '\0';
        monitor                = name;
        return true;
    }

    PHLWINDOW findWindow(int i, uint32_t pid, uint32_t actor) {
        auto& placed = g_placed[i];
        if (placed.pid == pid && placed.actor == actor) {
            if (const auto W = placed.window.lock(); W && W->mapped())
                return W;
        }
        const auto TITLE = std::format("anima-character-{}", actor);
        for (const auto& w : Desktop::windowState()->windows()) {
            if (!w || !w->mapped() || w->metadata().appID() != "anima-character" || w->metadata().title() != TITLE)
                continue;
            if (sc<uint32_t>(w->backend().pid()) != pid)
                continue;
            placed = {pid, actor, w};
            return w;
        }
        placed = {};
        return nullptr;
    }

    PHLWORKSPACE workspaceByID(int32_t id) {
        for (const auto& ws : State::Workspace::state()->workspaces()) {
            const auto WS = ws.lock();
            if (const auto NID = WS ? WS->numberedID() : std::nullopt; NID && sc<int32_t>(*NID) == id)
                return WS;
        }
        return nullptr;
    }

    PHLMONITOR monitorByName(const std::string& name) {
        for (const auto& m : State::monitorState()->monitors()) {
            if (m && m->m_name == name)
                return m;
        }
        return nullptr;
    }

    // Changing a window's workspace is heavier than moving it: done from the
    // event loop rather than in the middle of rendering.
    void moveToWorkspaces() {
        g_moveQueued = false;
        if (!g_shared)
            return;
        for (int i = 0; i < MAX_WINDOWS; ++i) {
            SWindowRecord rec{};
            std::string   mon;
            if (!readRecord(i, rec, mon))
                continue;
            const auto W  = findWindow(i, rec.pid, rec.actor);
            const auto WS = W ? workspaceByID(rec.workspace) : nullptr;
            if (W && WS && W->m_workspace != WS)
                Config::Actions::moveToWorkspace(WS, true, W);
        }
    }

    void placeWindows() {
        for (int i = 0; i < MAX_WINDOWS; ++i) {
            SWindowRecord rec{};
            std::string   mon;
            if (!readRecord(i, rec, mon))
                continue;
            const auto W = findWindow(i, rec.pid, rec.actor);
            if (!W || !W->isFloating())
                continue;
            const auto WS = workspaceByID(rec.workspace);
            if (WS && W->m_workspace != WS && !g_moveQueued) {
                g_moveQueued = true;
                g_pEventLoopManager->doLater(moveToWorkspaces);
            }
            const auto MON = monitorByName(mon);
            if (!MON)
                continue;
            const Vector2D TARGET = MON->m_position + Vector2D{rec.x, rec.y};
            if (W->position(Desktop::View::IGeometric::GEOMETRIC_GOAL) != TARGET)
                Config::Actions::move(TARGET, false, W);
            // where the layout put it, right away instead of animated
            W->positionAnimation()->warp();
        }
    }

    bool openShared() {
        const char* runtime = std::getenv("XDG_RUNTIME_DIR");
        if (!runtime)
            return false;
        const std::string PATH = std::string(runtime) + "/anima-hyprland.shm";
        const int         FD   = open(PATH.c_str(), O_RDWR | O_CREAT | O_CLOEXEC, 0600);
        if (FD < 0)
            return false;
        // never shrink: a reader's mapping past EOF would SIGBUS
        if (ftruncate(FD, SIZE) != 0) {
            close(FD);
            return false;
        }
        void* p = mmap(nullptr, SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, FD, 0);
        close(FD);
        if (p == MAP_FAILED)
            return false;
        g_shared = sc<SShared*>(p);
        std::memset(rc<char*>(g_shared) + 4, 0, SIZE - 4); // records from a previous session
        g_shared->version = VERSION;
        if (const char* sig = std::getenv("HYPRLAND_INSTANCE_SIGNATURE"))
            std::strncpy(g_shared->instance, sig, sizeof(g_shared->instance) - 1);
        g_shared->magic.store(MAGIC, std::memory_order_release);
        return true;
    }
}

APICALL EXPORT std::string PLUGIN_API_VERSION() {
    return HYPRLAND_API_VERSION;
}

APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    g_handle = handle;

    if (std::string{__hyprland_api_get_hash()} != __hyprland_api_get_client_hash())
        throw std::runtime_error("[anima-hypr] built against different Hyprland headers, rebuild it");

    if (!openShared())
        throw std::runtime_error("[anima-hypr] cannot create $XDG_RUNTIME_DIR/anima-hyprland.shm");

    // before each frame: the offset this frame is drawn with
    g_moveListener = Event::bus()->m_events.input.mouse.move.listen([](const Vector2D& pos, Event::SCallbackInfo&) { passThrough(pos); });
    g_listener     = Event::bus()->m_events.render.pre.listen([](auto mon) {
        publish(mon);
        placeWindows();
        keepFocus();
    });

    return {"anima-hypr", "Shares workspace slide offsets with anima's desktop characters", "anima", "1.0"};
}

APICALL EXPORT void PLUGIN_EXIT() {
    g_listener.reset();
    g_moveListener.reset();
    g_placed    = {};
    g_lastFocus = {};
    if (g_shared) {
        g_shared->magic.store(0, std::memory_order_release);
        munmap(g_shared, SIZE);
        g_shared = nullptr;
    }
}
