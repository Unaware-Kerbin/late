#!/usr/bin/env python3
"""Host-RAM refuse math for ov_worker (no OpenVINO import)."""

import unittest

from ov_worker import (
    HOST_RAM_VS_VRAM,
    IR_MISSING_START,
    convert_peak_bytes,
    parse_meminfo,
    refuse_host_weight_store,
)


class HostRamTests(unittest.TestCase):
    def test_parse_meminfo(self):
        mem = parse_meminfo(
            "MemTotal: 65000000 kB\nMemAvailable: 25165824 kB\nSwapTotal: 8388608 kB\nSwapFree: 102400 kB\n"
        )
        self.assertIsNotNone(mem)
        assert mem is not None
        self.assertEqual(mem["available"], 25165824 * 1024)
        self.assertEqual(mem["swap_free"], 102400 * 1024)

    def test_convert_peak_is_2_2x(self):
        self.assertEqual(convert_peak_bytes(10_000), 22_000)

    def test_swap_full_refuses(self):
        with self.assertRaises(RuntimeError) as ctx:
            refuse_host_weight_store(
                1_000_000_000,
                True,
                {"available": 24 * 1024**3, "swap_total": 8 * 1024**3, "swap_free": 100 * 1024**2},
            )
        self.assertIn(HOST_RAM_VS_VRAM, str(ctx.exception))
        self.assertIn("swap is full", str(ctx.exception))

    def test_gemma_convert_without_ir_refuses(self):
        with self.assertRaises(RuntimeError) as ctx:
            refuse_host_weight_store(
                10 * 1024**3,
                False,
                {"available": 24 * 1024**3, "swap_total": 8 * 1024**3, "swap_free": 4 * 1024**3},
            )
        self.assertIn(HOST_RAM_VS_VRAM, str(ctx.exception))
        self.assertIn("convert", str(ctx.exception))

    def test_missing_ir_unknown_size_refuses(self):
        with self.assertRaises(RuntimeError) as ctx:
            refuse_host_weight_store(
                0,
                False,
                {"available": 40 * 1024**3, "swap_total": 8 * 1024**3, "swap_free": 4 * 1024**3},
            )
        self.assertIn(HOST_RAM_VS_VRAM, str(ctx.exception))

    def test_tiny_ir_ok(self):
        refuse_host_weight_store(
            1_000_000_000,
            True,
            {"available": 40 * 1024**3, "swap_total": 8 * 1024**3, "swap_free": 4 * 1024**3},
        )

    def test_start_refuse_copy_mentions_your_computer(self):
        self.assertIn("your computer", IR_MISSING_START)
        self.assertIn("OpenVINO IR is missing", IR_MISSING_START)
        self.assertIn("MemAvailable", IR_MISSING_START)

    def test_vas_slop_covers_shared_library_maps(self):
        from ov_worker import CONVERT_VAS_SLOP_BYTES

        self.assertGreaterEqual(CONVERT_VAS_SLOP_BYTES, 1024 * 1024 * 1024)


if __name__ == "__main__":
    unittest.main()
