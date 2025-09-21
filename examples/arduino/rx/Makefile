BUILD   := build
BIN     := bin
TARGET  := $(BIN)/main

SRC     := src ../common/src
INC     := include ../common/include

CXX      := g++ -std=c++23
CXXFLAGS := -Wall \
            -g \
            -Wextra \
            -Wsign-conversion \
            -Wformat \
            -Wmissing-declarations \
            -Wpedantic \
            -D SIMULATE \
			-D NEXUS_LORA=\"$$HOME/nexus/lora\" \
            $(addprefix -I, $(INC))

SOURCES := $(foreach dir,$(SRC),$(wildcard $(dir)/*.cpp))
OBJECTS := $(patsubst $(SRC)/%.cpp,$(BUILD)/%.o,$(SOURCES))

RM := rm -rf

.PHONY: board upload simulate run clean

# Upload to device
board: upload simulate

upload:
	pio run -e aura --target upload --upload-port $(port)

monitor:
	pio device monitor -p $(port)

# Simulate on host PC
simulate: $(TARGET)
	./$(TARGET)

$(TARGET): $(OBJECTS)
	@mkdir -p $(BIN)
	$(CXX) $(CXXFLAGS) $(OBJECTS) -o $@

$(BUILD)/%.o: $(SRC)/%.cpp
	@mkdir -p $(BUILD)
	$(CXX) $(CXXFLAGS) -c $< -o $@

clean:
	$(RM) $(BIN) $(BUILD)
