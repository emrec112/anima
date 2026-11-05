# Build against the installed Hyprland headers (must match the running
# Hyprland exactly; rebuild after every Hyprland update).
CXXFLAGS ?= -O2
CXXFLAGS += -shared -fPIC -std=c++2b -Wno-c++11-narrowing
INCLUDES  = `pkg-config --cflags pixman-1 libdrm hyprland pangocairo libinput libudev wayland-server xkbcommon`

anima-hypr.so: main.cpp
	$(CXX) $(CXXFLAGS) $(INCLUDES) $< -o $@

clean:
	rm -f anima-hypr.so

.PHONY: clean
