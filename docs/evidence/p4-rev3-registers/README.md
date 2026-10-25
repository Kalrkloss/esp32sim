# ESP32-P4 rev < 3 against rev >= 3: the registers of the modelled blocks

What changes between ESP-IDF v5.5.4's `components/soc/esp32p4/register/hw_ver1` (rev < 3) and
`hw_ver3` (rev >= 3) in the register headers of the blocks `esp32p4` implements, keyed by
register offset and compared by field layout (bit position and width), names ignored: several
headers rename registers and fields without moving them (USB-Serial/JTAG, HP_SYSTEM's `SYS_`
prefix, the interrupt matrix's `CORE0_` prefix), which a name-keyed comparison counts as removed
and added.

    cd components/soc/esp32p4/register     # ESP-IDF v5.5.4
    python3 offdiff.py hw_ver1/soc hw_ver3/soc assist_debug_reg.h cache_reg.h sdmmc_reg.h \
      spi_mem_c_reg.h spi1_mem_c_reg.h spi_mem_s_reg.h spi1_mem_s_reg.h sha_reg.h \
      usb_serial_jtag_reg.h interrupt_core0_reg.h gpio_reg.h systimer_reg.h hp_system_reg.h \
      hp_sys_clkrst_reg.h lp_system_reg.h lp_clkrst_reg.h lp_timer_reg.h i2c_ana_mst_reg.h \
      efuse_reg.h > offdiff.txt

`uart_reg.h` and `timer_group_reg.h` define their registers through indexed macros the script
does not parse; their `#define` lines are identical between the two directories apart from one
line's whitespace. The USB-OTG core and the CLIC have no header in these directories.

Result ([offdiff.txt](offdiff.txt)):

- No register of these blocks moves.
- Unchanged: ASSIST_DEBUG, SDMMC, SHA, USB-Serial/JTAG, LP_CLKRST, LP_TIMER, I2C_ANA_MST (and UART,
  TIMG).
- Bits or registers added only: CACHE (a preload bit 7), the interrupt matrix (bits 6 and 7 of every
  map register, `SRC_PASS_IN_SEC` and `SRC_IN_SEC_FLAG` for the TEE, and new sources and status
  registers at the end), HP_SYS_CLKRST, HP_SYSTEM, SPI_MEM_C/S (interrupt bits 26-27, PMS attribute
  bits, new registers).
- Removed: SYSTIMER `CONF` bit 0, LP_SYSTEM `SYS_CTRL` bits 22-26, HP_SYSTEM `RNG_CFG` bits 24-31,
  SPI_MEM_C/S `CTRL1` bit 23, GPIO input-select registers for signals 138 and 139.
- Resized: SPI_MEM_C/S `FSM` lock delay (5 to 12 bits), `XTS_PHYSICAL_ADDRESS` (26 to 30 bits).
- eFuse: the repeat-data words, `RD_MAC_SYS_5` and system part 2 data 3/6/7 are laid out anew; the
  MAC words and `RD_MAC_SYS_2` (wafer and block revisions), which the model writes, are not.

None of it is in a field the model gives a meaning to; the interrupt matrix keeps six bits of each
map register, so the TEE flags read back 0. This is why hello_world and ESP-Hosted run on the rev
>= 3 model with the rev < 3 peripherals.
