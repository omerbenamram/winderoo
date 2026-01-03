#!/usr/bin/env python3
"""Serial monitor for ESP32 with optional device reset."""

import serial
import signal
import sys
import time


def main():
    port_path = sys.argv[1] if len(sys.argv) > 1 else "/dev/cu.usbserial-0001"
    duration = int(sys.argv[2]) if len(sys.argv) > 2 else 30
    no_reset = "--no-reset" in sys.argv

    # Handle SIGPIPE gracefully (e.g., when piped to head)
    signal.signal(signal.SIGPIPE, signal.SIG_DFL)

    # Open port with proper settings - always reset serial state
    port = serial.Serial(
        port_path,
        baudrate=115200,
        bytesize=serial.EIGHTBITS,
        parity=serial.PARITY_NONE,
        stopbits=serial.STOPBITS_ONE,
        timeout=0.1,
        xonxoff=False,
        rtscts=False,
        dsrdtr=False,
    )

    # Flush any stale data in buffers
    port.reset_input_buffer()
    port.reset_output_buffer()

    if not no_reset:
        # Reset device via RTS toggle (EN pin on most ESP32 dev boards)
        port.dtr = False
        port.rts = True
        time.sleep(0.1)
        port.rts = False
        time.sleep(0.1)  # Give device time to start booting

    start = time.time()
    try:
        while time.time() - start < duration:
            data = port.read(4096)
            if data:
                sys.stdout.write(data.decode("utf-8", errors="replace"))
                sys.stdout.flush()
    except KeyboardInterrupt:
        pass
    except BrokenPipeError:
        # Handle pipe closed (e.g., head finished)
        pass
    finally:
        port.close()


if __name__ == "__main__":
    main()
