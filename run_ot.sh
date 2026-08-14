#!/bin/bash
# Jalankan simulasi OpenTitan dengan DBG_ELAB untuk progress real-time
cd /home/whale-d/maria
export DBG_ELAB=1
timeout 1800 ./target/release/maria --filelist opentitan_rtl.f -T 1 > /tmp/ot_dbg3.log 2>&1
echo "SIM_EXIT=$?" >> /tmp/ot_dbg3.log
