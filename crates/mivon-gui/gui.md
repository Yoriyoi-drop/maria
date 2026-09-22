Mivon GUI: Fitur dan Ekosistem

Menurutku, Mivon GUI sebaiknya dibangun sebagai lingkungan pengembangan dan debugging hardware yang terintegrasi, bukan sekadar GUI untuk menjalankan simulator.
Fokus utamanya adalah membantu developer memahami mengapa simulasi gagal, menelusuri sumber masalah hingga ke RTL, menguji perbaikan, dan memastikan perubahan tidak merusak desain lain.
Karena Mivon berbasis Rust dan diarahkan ke simulasi SystemVerilog, GUI-nya bisa dirancang seperti IDE profesional yang menyatukan editor, simulator, debugger, waveform, coverage, dan sistem pengujian.
1. Struktur utama GUI
Mivon Workbench
Satu workspace untuk pengembangan RTL, debugging, simulasi, dan verifikasi.
01
Project Explorer
Struktur repository, filelist, modul, dan dependency.
02
RTL Editor
Editor SystemVerilog dengan navigasi simbol dan diagnostic.
03
Simulation Control
Build, elaboration, run, pause, reset, dan konfigurasi.
04
Debugger
Breakpoint, step, call stack, variabel, dan event.
05
Waveform
Analisis perubahan sinyal terhadap waktu.
06
Trace & Coverage
Penelusuran kejadian, coverage, dan verifikasi.
07
Console & Logs
Output terstruktur untuk build, simulasi, dan fuzzing.
08
Diagnostics
Lokasi error, hubungan antarfile, dan navigasi ke sumber.
2. Fitur inti yang perlu ada
A. Project Explorer dan RTL Editor
Project Explorer
Navigasi file RTL, testbench, include, dan konfigurasi.
Hierarki modul dan instance.
Pencarian simbol lintas repository.
Deteksi dependency dan file yang berubah.
Dukungan multi-project dan workspace.
RTL Editor
Syntax highlighting SystemVerilog.
Go to Definition, Find References, dan symbol outline.
Diagnostic langsung dengan lokasi baris dan kolom.
Hover informasi tipe, parameter, dan deklarasi.
Integrasi language server.
Perbandingan perubahan kode (diff).
Navigasi dari error ke baris RTL terkait.
Fitur penting: saat sebuah sinyal dipilih di waveform, editor otomatis membuka deklarasi sinyal tersebut.
B. Interactive Debugger
Ini seharusnya menjadi salah satu pusat utama Mivon.
Fitur
Kegunaan
Breakpoint
Menghentikan eksekusi pada lokasi tertentu.
Conditional breakpoint
Berhenti ketika kondisi tertentu terpenuhi
Step
Menelusuri eksekusi secara bertahap.
Continue / Pause
Melanjutkan atau menghentikan sementara simulasi.
Call stack
Melihat jalur pemanggilan dan konteks eksekusi.
Variable inspector
Memeriksa nilai variabel, parameter, dan objek yang tersedia.
Watch expressions
Memantau ekspresi selama debugging.
Event inspection
Melihat kejadian yang memicu perubahan nilai.
Source mapping
Menghubungkan lokasi eksekusi dengan kode RTL.
Fitur lanjutan: reverse debugging, jika simulator mendukung checkpoint dan pemulihan state secara benar. Jangan sekadar membuat tombol mundur yang tampil meyakinkan tetapi tidak mampu mengembalikan state simulasi.
C. Waveform Analyzer
Waveform bukan hanya tempat melihat gelombang 0 dan 1. Ia harus menjadi alat investigasi.
Tambah atau hapus sinyal dari hierarki.
Grouping sinyal berdasarkan modul.
Bus display dalam binary, hexadecimal, decimal, dan signed.
Zoom, pan, cursor, dan time ruler.
Pengukuran selisih waktu antar-event.
Pencarian transisi nilai.
Perbandingan waveform antar-run.
Navigasi langsung ke source dan debugger.
Ekspor waveform ke format yang didukung, misalnya VCD atau FST.
Fitur pembeda: pilih satu event di waveform, lalu Mivon menampilkan konteks simulasi, nilai sebelum dan sesudah, serta lokasi sumber yang tersedia.
D. Trace dan Event Timeline
Trace menjawab pertanyaan yang tidak selalu bisa dijawab waveform: urutan kejadian apa yang membawa simulasi ke kondisi gagal?
Fitur yang dibutuhkan:
Timeline event per modul.
Filter berdasarkan sinyal, modul, waktu, dan jenis event.
Navigasi dari assertion failure ke event sebelumnya.
Relasi sebab-akibat jika dapat dibuktikan dari data eksekusi.
Penanda race, konflik, atau event mencurigakan jika didukung engine.
Penyimpanan trace untuk investigasi setelah run selesai.
Mivon perlu membedakan urutan kejadian yang benar-benar tercatat dengan dugaan penyebab yang dihasilkan analisis. Jangan mencampur fakta dengan tebakan hanya demi UI terlihat pintar.
3. Ekosistem verifikasi dan testing
Ini bagian yang membuat Mivon lebih dari sekadar editor dengan tombol Run.
Verification Center
Semua jalur pengujian terhubung ke hasil, diagnostic, dan artefak yang bisa diperiksa.
Build & Elaboration
Compile RTL, resolve dependency, elaborasi hierarchy, dan laporan waktu serta penggunaan resource.
Simulation Runner
Menjalankan testbench, konfigurasi top, seed, parameter, dan mode simulasi.
Assertion & Property Testing
Memeriksa assertion, property, dan kegagalan temporal sesuai dukungan engine.
Coverage Analyzer
Line, toggle, branch, expression, FSM, dan functional coverage sesuai instrumentasi yang tersedia.
Fuzzing Center
Generate test, jalankan seed, minimisasi kasus gagal, dan kelola regression corpus.
Regression Manager
Jalankan kumpulan test, bandingkan hasil, dan identifikasi perubahan yang menyebabkan regression.
Coverage Analyzer
GUI coverage idealnya mempunyai tiga tingkat:
Project Coverage
Ringkasan seluruh desain dan test suite.
Global
Module Coverage
Coverage per modul, instance, atau hierarki.
Per modul
Source Coverage
Highlight baris RTL dan kondisi yang belum tercakup.
Per baris
Tambahkan fitur untuk melihat test mana yang menghasilkan coverage tertentu, bukan cuma menampilkan persentase besar yang membuat orang merasa produktif.
Fuzzing Center untuk Mivon
Karena Mivon juga dikembangkan bersama Maria-fuzz, GUI-nya dapat menyediakan pusat fuzzing yang terhubung langsung dengan simulator.
Fitur
Penjelasan
Campaign Manager
Membuat, menjalankan, menghentikan, dan melanjutkan campaign.
Seed Corpus
Mengelola input awal dan test case hasil mutasi.
Generator Config
Mengatur jenis test, batas ukuran, dan strategi mutasi.
Worker Monitor
Memantau worker, thread, penggunaan CPU/RAM, dan throughput.
Failure Triage
Mengelompokkan crash, assertion failure, timeout, dan mismatch.
Testcase Minimizer
Mengurangi kasus gagal tanpa menghilangkan kondisi pemicunya.
Regression Corpus
Menyimpan kasus yang harus tetap diuji setelah perubahan.
Coverage Guidance
Mengarahkan generasi berdasarkan coverage yang benar-benar tersedia.
Satu hal penting: pisahkan fuzzing untuk menemukan bug di simulator dari fuzzing untuk memverifikasi desain RTL.
Keduanya memakai sebagian infrastruktur yang sama, tetapi target, oracle, klasifikasi kegagalan, dan cara menentukan keberhasilan berbeda.
4. Mivon Differential Verification
Fitur ini sangat relevan untuk menguji kompatibilitas simulator.
Multi-Engine Comparison
Satu test case, beberapa engine, lalu bandingkan hasil yang dapat dibandingkan secara sah.
RTL / Testbench / Seed
Mivon
Engine internal
Verilator
External reference
Simulator lain
Opsional, sesuai lisensi
Result Comparator
Normalisasi output, bandingkan event dan nilai sinyal, tandai mismatch, simpan reproducer.
Fitur yang perlu disediakan:
Adapter untuk simulator eksternal.
Konfigurasi toolchain per engine.
Perbandingan output dan assertion.
Perbandingan waveform pada titik observasi yang kompatibel.
Pengelompokan mismatch berdasarkan lokasi dan waktu.
Reproducer minimal untuk bug.
Laporan perbedaan semantik dan dukungan fitur.
Hasil yang berbeda tidak otomatis berarti Mivon salah. Perlu diperiksa apakah input, konfigurasi, semantik yang digunakan, dan titik observasinya memang setara.
5. Ekosistem plugin dan integrasi
Mivon GUI sebaiknya punya arsitektur modular. Fitur inti tidak perlu bergantung pada satu engine atau satu vendor.
Mivon Platform Architecture
Mivon GUI
Editor · Debugger · Waveform · Coverage · Testing
Mivon Core Services
Project · Job Scheduler · Diagnostics · Artifact Manager · Plugin API
Simulation Engine
Compile, elaboration, run, event execution
Debug Services
Breakpoints, state inspection, trace
Verification
Assertions, coverage, regression, fuzzing
External Adapters
Verilator, waveform tools, CI, toolchains
Plugin yang bisa dikembangkan
Plugin
Fungsi
Simulator Adapter
Menghubungkan engine simulasi lain.
Waveform Adapter
Membaca format waveform tambahan.
Lint Integration
Menampilkan hasil lint dari tool eksternal.
Synthesis Reports
Membaca laporan sintesis dan timing.




Formal Verification

	

Integrasi formal tools bila tersedia.




Git Integration

	

Diff, branch, commit, dan regression terhadap perubahan.




CI Integration

	

Mengakses hasil build dan test dari GitHub Actions.




Board / FPGA Tools

	

Integrasi toolchain hardware eksternal jika dibutuhkan.

Plugin API sebaiknya mempunyai versi, izin akses, batasan resource, serta format komunikasi yang stabil. Jangan sampai update plugin pihak ketiga merusak seluruh workspace.

6. Integrasi GitHub dan CI/CD

Mivon dapat terhubung dengan pipeline GitHub Actions untuk membawa hasil pengujian dari mesin lokal ke CI, lalu kembali ke GUI.

Developer Workflow
Developer mengubah RTL atau testbench di Mivon.
Mivon menjalankan build, simulasi, dan test lokal.
Developer mengirim perubahan ke GitHub.
GitHub Actions menjalankan regression, fuzzing terpilih, dan differential tests.
Mivon mengambil status, log, diagnostic, dan artefak yang tersedia.
Developer membuka hasil gagal langsung dari workspace untuk investigasi.
Fitur GUI terkait CI:
Status workflow dan commit.
Daftar job yang gagal.
Perbandingan hasil antar-commit.
Download dan buka artefak test.
Link ke log GitHub Actions.
Regression dashboard.
Penanda versi stabil dan eksperimen.
Untuk repositori Mivon, ini bisa menjadi jalur kontribusi yang jelas: contributor mengembangkan fitur, CI memvalidasi perubahan, dan GUI mempermudah investigasi ketika pengujian gagal.
7. Fitur lanjutan kelas industri
Fitur berikut bukan keharusan untuk versi pertama, tetapi layak menjadi bagian roadmap.
Performance Profiler
Mengidentifikasi modul, proses, event, dan operasi yang paling banyak mengonsumsi waktu. Cocok untuk investigasi bottleneck simulator.
Resource Monitor
Memantau CPU, RAM, thread, penggunaan disk, ukuran trace, dan antrean pekerjaan.
Regression Intelligence
Membandingkan hasil pengujian berdasarkan commit, konfigurasi, seed, dan versi engine untuk menemukan perubahan perilaku.
Session & Artifact Manager
Menyimpan konfigurasi run, breakpoint, watchlist, log, waveform, trace, dan hasil investigasi agar bug bisa direproduksi.
Fitur kolaborasi tim
Shared run configuration.
Export dan import debug session.
Laporan bug dengan reproducer.
Catatan investigasi per failure.
Riwayat regression.
Role dan permission jika nanti ada server tim.
Pengelolaan artefak dengan batas penyimpanan.
8. Pembagian versi pengembangan
Jangan mencoba membangun seluruh ekosistem sekaligus. Itu resep klasik untuk menghasilkan 40 panel UI yang semuanya setengah jadi.
Roadmap Mivon GUI
Usulan
Tahap 1 · Core Workbench
Editor + Project + Simulation + Console
Fondasi workspace, konfigurasi run, diagnostic, dan pengelolaan proyek.
Tahap 2 · Debugging
Debugger + Waveform + Trace
Breakpoint, state inspection, event navigation, dan korelasi source-to-waveform.
Tahap 3 · Verification
Coverage + Fuzzing + Regression
Campaign manager, failure triage, testcase minimization, dan laporan coverage.
Tahap 4 · Ecosystem
Plugins + CI + External Engines
Adapter, integrasi GitHub Actions, differential testing, dan dukungan toolchain tambahan.
Tahap 5 · Advanced Analysis
Profiling + Formal + Team Workflows
Analisis performa, integrasi formal, dan kolaborasi tim yang lebih luas.
9. Rekomendasi susunan sidebar
Ini susunan navigasi yang menurutku paling masuk akal untuk Mivon:
Mivon
GUI concept
Project
Design
Debug
Waveform
Trace
Verification
Fuzzing
Profiler
Build & CI
Settings
WORKSPACE MODULE
Debug
Debugger, breakpoints, variables, call stack.
CONTEXT ACTIONS
Run / Pause / Step
Breakpoints · Variables · Call Stack
Pilih modul untuk melihat ringkasan fungsi.