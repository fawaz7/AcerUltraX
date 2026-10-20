/* Test DLL for pe-loader: exercises relocations, imports, implicit TLS, DllMain and the win64 ABI.
 * Built without a C runtime: clang --target=x86_64-pc-windows-msvc + lld-link (see ../loader.rs). */

typedef unsigned long DWORD;
typedef int BOOL;
typedef void *HANDLE;
typedef unsigned long long SIZE_T;

__declspec(dllimport) HANDLE GetProcessHeap(void);
__declspec(dllimport) void *HeapAlloc(HANDLE heap, DWORD flags, SIZE_T size);
__declspec(dllimport) BOOL HeapFree(HANDLE heap, DWORD flags, void *p);
__declspec(dllimport) DWORD GetLastError(void);
__declspec(dllimport) void SetLastError(DWORD e);
__declspec(dllimport) DWORD FlsAlloc(void *callback);
__declspec(dllimport) BOOL FlsSetValue(DWORD i, void *v);
__declspec(dllimport) void *FlsGetValue(DWORD i);

/* Implicit TLS, set up the way the MSVC runtime's tlssup does. */
unsigned long _tls_index = 0;
#pragma section(".tls", read, write)
#pragma section(".tls$ZZZ", read, write)
__declspec(allocate(".tls")) char _tls_start = 0;
__declspec(allocate(".tls$ZZZ")) char _tls_end = 0;
#pragma section(".CRT$XLA", read)
#pragma section(".CRT$XLZ", read)
__declspec(allocate(".CRT$XLA")) void *__xl_a = 0;
__declspec(allocate(".CRT$XLZ")) void *__xl_z = 0;
typedef struct {
    unsigned long long start, end, index, callbacks;
    DWORD zero_fill, characteristics;
} TLS_DIRECTORY;
const TLS_DIRECTORY _tls_used = {(unsigned long long)&_tls_start, (unsigned long long)&_tls_end,
                                 (unsigned long long)&_tls_index, (unsigned long long)(&__xl_a + 1), 0, 0};

int _fltused = 0; /* the compiler references it whenever floating point is used */

__declspec(thread) int tls_value = 1234;

static int process_attach, thread_attach;
static const char *const names[] = {"alpha", "beta"}; /* pointers that need relocating */
static DWORD fls_slot;

BOOL DllMain(HANDLE inst, DWORD reason, void *reserved)
{
    if (reason == 1) {
        process_attach++;
        fls_slot = FlsAlloc(0);
    }
    if (reason == 2) thread_attach++;
    return 1;
}

__declspec(dllexport) int attach_counts(void) { return process_attach * 1000 + thread_attach; }
__declspec(dllexport) int tls_get(void) { return tls_value; }
__declspec(dllexport) void tls_set(int v) { tls_value = v; }
__declspec(dllexport) const char *name_at(int i) { return names[i]; }
__declspec(dllexport) double mul_add(double a, double b, float c, int d) { return a * b + c + d; }

__declspec(dllexport) int heap_round_trip(int n)
{
    unsigned char *p = HeapAlloc(GetProcessHeap(), 8 /* HEAP_ZERO_MEMORY */, n);
    int ok = p != 0 && ((SIZE_T)p & 15) == 0;
    for (int i = 0; ok && i < n; i++) ok = p[i] == 0;
    HeapFree(GetProcessHeap(), 0, p);
    return ok;
}

__declspec(dllexport) DWORD last_error_round_trip(DWORD v)
{
    SetLastError(v);
    return GetLastError();
}

__declspec(dllexport) SIZE_T fls_round_trip(SIZE_T v)
{
    FlsSetValue(fls_slot, (void *)v);
    return (SIZE_T)FlsGetValue(fls_slot);
}
