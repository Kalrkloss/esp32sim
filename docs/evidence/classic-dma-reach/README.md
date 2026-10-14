# Classic DMA address reach

Classic DMA uses the internal DRAM data window [0x3ffae000, 0x40000000).
The CPU's ROM-integrity backing prefix at 0x3ffadafc is not part of that
window. IRAM aliases, mask ROM, RTC memory, flash mappings, external RAM and
registers are not DMA-capable through this interface.

Sources:

- ESP-IDF v5.5.4, `components/soc/esp32/include/soc/soc.h:203-205`:
  `SOC_DMA_LOW`/`SOC_DMA_HIGH` define the DMA-accessible memory region.
- ESP-IDF v4.4.8, the same file at lines 266-268: identical bounds.
- ESP-IDF v5.5.4,
  `components/esp_hw_support/include/esp_memory_utils.h:224-227`:
  `esp_ptr_dma_capable` checks the inclusive lower and exclusive upper bounds.
- ESP-IDF v5.5.4, `components/soc/esp32/include/soc/soc.h:195-201`:
  D/IRAM has separate data/instruction windows; use the DRAM window for DMA.

The shared `bus::dma_range` checks the entire requested span and returns
DRAM backing offsets. I2S, SPI TX/RX and Wi-Fi descriptor/payload accesses
use it. SPI payloads are checked before any transfer bytes are read or
written. A word crossing the upper boundary is rejected before writeback.
The CPU memory map and its reset-integrity prefix are unchanged.

Regression-first: with the new tests and the old implementation, I2S
passed while SPI and Wi-Fi failed at 0x3ffadafc. With the shared rule,
all three pass. Tests include both ends of DRAM, the CPU-only prefix,
IRAM/ROM/RTC addresses, crossing words and address/length overflow.

Reproduce the engine tests:

    cargo +1.99.0 test --release -p esp32 dma_reach

Existing I2S, SPI and Wi-Fi end-to-end tests also exercise valid transfers,
completion/error reporting and DMA buffers. The range rule derives from
public headers; no silicon validation or Wi-Fi hardware fault-bit behavior
is claimed. No changes to clocks, tick gating, scheduler or struct layout.
