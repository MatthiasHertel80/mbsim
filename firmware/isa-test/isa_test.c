/*
 * ISA differential test for the ARMv7E-M + FPv4-SP core.
 *
 * Executes instructions on deterministic pseudo-random and edge-case operands and prints a
 * digest per group over semihosting. Run the same image on the micro:bit V2 simulator and on
 * Arm's FVP_MPS2_Cortex-M4 and compare the outputs; any difference is a simulator bug.
 */
#include <stdint.h>

/* ------------------------------------------------------------------ semihosting output */

static int semihost(int op, void *arg) {
  register int r0 __asm("r0") = op;
  register void *r1 __asm("r1") = arg;
  __asm volatile("bkpt 0xAB" : "+r"(r0) : "r"(r1) : "memory");
  return r0;
}

static void puts_(const char *s) { semihost(0x04, (void *)s); }

void semihost_exit(int code) {
  uint32_t block[2] = { 0x20026, (uint32_t)code };
  semihost(0x20, block);   /* SYS_EXIT_EXTENDED */
  semihost(0x18, (void *)0x20026);
  for (;;) {}
}

static void put_hex(uint32_t v) {
  char b[11] = "0x00000000";
  for (int i = 0; i < 8; i++) b[9 - i] = "0123456789abcdef"[(v >> (4 * i)) & 15];
  puts_(b);
}

static void report(const char *name, uint32_t digest) {
  puts_(name);
  puts_(": ");
  put_hex(digest);
  puts_("\n");
}

/* ------------------------------------------------------------------ helpers */

static uint32_t digest;
static void mix(uint32_t v) {
  for (int i = 0; i < 4; i++) {
    digest ^= (v >> (8 * i)) & 0xFF;
    digest *= 16777619u;
  }
}

static uint32_t rng_state = 0x12345678;
static uint32_t rnd(void) {
  uint32_t x = rng_state;
  x ^= x << 13; x ^= x >> 17; x ^= x << 5;
  rng_state = x;
  return x;
}

static const uint32_t edges[] = {
  0, 1, 2, 0x7F, 0x80, 0xFF, 0x7FFF, 0x8000, 0xFFFF, 0x10000, 0x7FFFFFFF, 0x80000000, 0x80000001,
  0xFFFFFFFE, 0xFFFFFFFF, 0x55555555, 0xAAAAAAAA, 0x00FF00FF, 0xFF00FF00, 31, 32, 33,
};
#define NEDGE (sizeof(edges) / sizeof(edges[0]))
#define N 400

static uint32_t operand(int i, int which) {
  if (i < (int)(NEDGE * NEDGE)) return which ? edges[i % NEDGE] : edges[i / NEDGE];
  return rnd();
}

#define BEGIN(name) do { digest = 2166136261u; rng_state = 0x9E3779B9u ^ (uint32_t)(sizeof(name)); } while (0)

/* binary operation with flags in and out (APSR NZCVQ and GE) */
#define OP2F(fn, insn)                                                                         \
  static uint32_t fn(uint32_t a, uint32_t b, uint32_t fin, uint32_t *fout) {                   \
    uint32_t r, f;                                                                             \
    __asm volatile("msr APSR_nzcvqg, %[fi]\n\t" insn "\n\tmrs %[f], APSR"                      \
                   : [r] "=&r"(r), [f] "=&r"(f) : [a] "r"(a), [b] "r"(b), [fi] "r"(fin) : "cc"); \
    *fout = f;                                                                                 \
    return r;                                                                                  \
  }

OP2F(op_adds, "adds %[r], %[a], %[b]")
OP2F(op_adcs, "adcs.w %[r], %[a], %[b]")
OP2F(op_subs, "subs %[r], %[a], %[b]")
OP2F(op_sbcs, "sbcs.w %[r], %[a], %[b]")
OP2F(op_rsbs, "rsbs.w %[r], %[a], %[b]")
OP2F(op_cmp, "cmp %[a], %[b]\n\tmov %[r], #0")
OP2F(op_cmn, "cmn %[a], %[b]\n\tmov %[r], #0")
OP2F(op_ands_lsl, "ands.w %[r], %[a], %[b], lsl #3")
OP2F(op_orrs_lsr, "orrs.w %[r], %[a], %[b], lsr #1")
OP2F(op_eors_asr, "eors.w %[r], %[a], %[b], asr #31")
OP2F(op_bics_ror, "bics.w %[r], %[a], %[b], ror #7")
OP2F(op_orns_rrx, "orns %[r], %[a], %[b], rrx")
OP2F(op_mvns_lsl, "mvns.w %[r], %[b], lsl #1")
OP2F(op_tst, "tst.w %[a], %[b], lsl #31\n\tmov %[r], #0")
OP2F(op_teq, "teq.w %[a], %[b], asr #2\n\tmov %[r], #0")
OP2F(op_adds_imm, "adds.w %[r], %[a], #0xFF000000")
OP2F(op_subs_imm, "subs.w %[r], %[a], #0x00AB00AB")
OP2F(op_ands_imm, "ands %[r], %[a], #0x80000000")
OP2F(op_lsls, "lsls.w %[r], %[a], %[b]")
OP2F(op_lsrs, "lsrs.w %[r], %[a], %[b]")
OP2F(op_asrs, "asrs.w %[r], %[a], %[b]")
OP2F(op_rors, "rors.w %[r], %[a], %[b]")
OP2F(op_lsls_imm, "lsls %[r], %[a], #17")
OP2F(op_lsrs_imm, "lsrs %[r], %[a], #32")
OP2F(op_asrs_imm, "asrs %[r], %[a], #32")
OP2F(op_rrxs, "rrxs %[r], %[a]")
static uint32_t op_muls(uint32_t a, uint32_t b, uint32_t fin, uint32_t *fout) {
  register uint32_t r __asm("r3");
  register uint32_t bb __asm("r2") = b;
  uint32_t f;
  __asm volatile("msr APSR_nzcvqg, %[fi]\n\tmovs r3, %[a]\n\tmuls r3, r2, r3\n\tmrs %[f], APSR"
                 : "=&l"(r), [f] "=&r"(f) : [a] "r"(a), "l"(bb), [fi] "r"(fin) : "cc");
  *fout = f;
  return r;
}
OP2F(op_qadd, "qadd %[r], %[a], %[b]")
OP2F(op_qsub, "qsub %[r], %[a], %[b]")
OP2F(op_qdadd, "qdadd %[r], %[a], %[b]")
OP2F(op_qdsub, "qdsub %[r], %[a], %[b]")
OP2F(op_ssat, "ssat %[r], #9, %[a], lsl #3")
OP2F(op_ssat2, "ssat %[r], #32, %[a], asr #5")
OP2F(op_usat, "usat %[r], #7, %[a], asr #2")
OP2F(op_usat0, "usat %[r], #0, %[a]")
OP2F(op_ssat16, "ssat16 %[r], #5, %[a]")
OP2F(op_usat16, "usat16 %[r], #9, %[a]")
OP2F(op_sadd16, "sadd16 %[r], %[a], %[b]")
OP2F(op_ssub16, "ssub16 %[r], %[a], %[b]")
OP2F(op_sasx, "sasx %[r], %[a], %[b]")
OP2F(op_ssax, "ssax %[r], %[a], %[b]")
OP2F(op_sadd8, "sadd8 %[r], %[a], %[b]")
OP2F(op_ssub8, "ssub8 %[r], %[a], %[b]")
OP2F(op_uadd16, "uadd16 %[r], %[a], %[b]")
OP2F(op_usub16, "usub16 %[r], %[a], %[b]")
OP2F(op_uasx, "uasx %[r], %[a], %[b]")
OP2F(op_usax, "usax %[r], %[a], %[b]")
OP2F(op_uadd8, "uadd8 %[r], %[a], %[b]")
OP2F(op_usub8, "usub8 %[r], %[a], %[b]")
OP2F(op_qadd16, "qadd16 %[r], %[a], %[b]")
OP2F(op_qsub16, "qsub16 %[r], %[a], %[b]")
OP2F(op_qasx, "qasx %[r], %[a], %[b]")
OP2F(op_qsax, "qsax %[r], %[a], %[b]")
OP2F(op_qadd8, "qadd8 %[r], %[a], %[b]")
OP2F(op_qsub8, "qsub8 %[r], %[a], %[b]")
OP2F(op_uqadd16, "uqadd16 %[r], %[a], %[b]")
OP2F(op_uqsub16, "uqsub16 %[r], %[a], %[b]")
OP2F(op_uqasx, "uqasx %[r], %[a], %[b]")
OP2F(op_uqsax, "uqsax %[r], %[a], %[b]")
OP2F(op_uqadd8, "uqadd8 %[r], %[a], %[b]")
OP2F(op_uqsub8, "uqsub8 %[r], %[a], %[b]")
OP2F(op_shadd16, "shadd16 %[r], %[a], %[b]")
OP2F(op_shsub16, "shsub16 %[r], %[a], %[b]")
OP2F(op_shasx, "shasx %[r], %[a], %[b]")
OP2F(op_shsax, "shsax %[r], %[a], %[b]")
OP2F(op_shadd8, "shadd8 %[r], %[a], %[b]")
OP2F(op_shsub8, "shsub8 %[r], %[a], %[b]")
OP2F(op_uhadd16, "uhadd16 %[r], %[a], %[b]")
OP2F(op_uhsub16, "uhsub16 %[r], %[a], %[b]")
OP2F(op_uhasx, "uhasx %[r], %[a], %[b]")
OP2F(op_uhsax, "uhsax %[r], %[a], %[b]")
OP2F(op_uhadd8, "uhadd8 %[r], %[a], %[b]")
OP2F(op_uhsub8, "uhsub8 %[r], %[a], %[b]")
OP2F(op_sel, "sel %[r], %[a], %[b]")
OP2F(op_smulbb, "smulbb %[r], %[a], %[b]")
OP2F(op_smultb, "smultb %[r], %[a], %[b]")
OP2F(op_smulbt, "smulbt %[r], %[a], %[b]")
OP2F(op_smultt, "smultt %[r], %[a], %[b]")
OP2F(op_smlabb, "smlabb %[r], %[a], %[b], %[a]")
OP2F(op_smlatt, "smlatt %[r], %[a], %[b], %[b]")
OP2F(op_smulwb, "smulwb %[r], %[a], %[b]")
OP2F(op_smulwt, "smulwt %[r], %[a], %[b]")
OP2F(op_smlawb, "smlawb %[r], %[a], %[b], %[a]")
OP2F(op_smlawt, "smlawt %[r], %[a], %[b], %[b]")
OP2F(op_smuad, "smuad %[r], %[a], %[b]")
OP2F(op_smuadx, "smuadx %[r], %[a], %[b]")
OP2F(op_smusd, "smusd %[r], %[a], %[b]")
OP2F(op_smusdx, "smusdx %[r], %[a], %[b]")
OP2F(op_smlad, "smlad %[r], %[a], %[b], %[a]")
OP2F(op_smladx, "smladx %[r], %[a], %[b], %[b]")
OP2F(op_smlsd, "smlsd %[r], %[a], %[b], %[a]")
OP2F(op_smlsdx, "smlsdx %[r], %[a], %[b], %[b]")
OP2F(op_smmul, "smmul %[r], %[a], %[b]")
OP2F(op_smmulr, "smmulr %[r], %[a], %[b]")
OP2F(op_smmla, "smmla %[r], %[a], %[b], %[a]")
OP2F(op_smmlar, "smmlar %[r], %[a], %[b], %[b]")
OP2F(op_smmls, "smmls %[r], %[a], %[b], %[a]")
OP2F(op_smmlsr, "smmlsr %[r], %[a], %[b], %[b]")
OP2F(op_usad8, "usad8 %[r], %[a], %[b]")
OP2F(op_usada8, "usada8 %[r], %[a], %[b], %[a]")
OP2F(op_mla, "mla %[r], %[a], %[b], %[a]")
OP2F(op_mls, "mls %[r], %[a], %[b], %[b]")
OP2F(op_sdiv, "sdiv %[r], %[a], %[b]")
OP2F(op_udiv, "udiv %[r], %[a], %[b]")
OP2F(op_clz, "clz %[r], %[a]")
OP2F(op_rbit, "rbit %[r], %[a]")
OP2F(op_rev, "rev %[r], %[a]")
OP2F(op_rev16, "rev16.w %[r], %[a]")
OP2F(op_revsh, "revsh.w %[r], %[a]")
OP2F(op_ubfx, "ubfx %[r], %[a], #5, #11")
OP2F(op_sbfx, "sbfx %[r], %[a], #13, #19")
OP2F(op_bfi, "mov %[r], %[b]\n\tbfi %[r], %[a], #7, #13")
OP2F(op_bfc, "mov %[r], %[a]\n\tbfc %[r], #3, #26")
OP2F(op_uxtab, "uxtab %[r], %[a], %[b], ror #8")
OP2F(op_sxtab, "sxtab %[r], %[a], %[b], ror #16")
OP2F(op_uxtah, "uxtah %[r], %[a], %[b], ror #24")
OP2F(op_sxtah, "sxtah %[r], %[a], %[b]")
OP2F(op_uxtab16, "uxtab16 %[r], %[a], %[b], ror #8")
OP2F(op_sxtab16, "sxtab16 %[r], %[a], %[b]")
OP2F(op_uxtb16, "uxtb16 %[r], %[b], ror #16")
OP2F(op_sxtb16, "sxtb16 %[r], %[b], ror #24")
OP2F(op_pkhbt, "pkhbt %[r], %[a], %[b], lsl #9")
OP2F(op_pkhtb, "pkhtb %[r], %[a], %[b], asr #12")
OP2F(op_movw_movt, "movw %[r], #0x1234\n\tmovt %[r], #0xABCD\n\tadd %[r], %[a]")
OP2F(op_it, "cmp %[a], %[b]\n\tite hi\n\tmovhi %[r], #1\n\tmovls %[r], #2\n\titt lt\n\taddlt %[r], %[r], #16\n\taddslt %[r], %[r], #0")
OP2F(op_it16, "mov %[r], %[a]\n\tcmp %[a], %[b]\n\titttt ne\n\taddne %[r], %[r], #1\n\tlslne %[r], %[r], #2\n\tsubne %[r], %[r], %[b]\n\teorne %[r], %[r], %[a]")

typedef uint32_t (*op2f_t)(uint32_t, uint32_t, uint32_t, uint32_t *);

static void run_op2f(const char *name, op2f_t f, int divide) {
  BEGIN(name);
  for (int i = 0; i < N + (int)(NEDGE * NEDGE); i++) {
    uint32_t a = operand(i, 0), b = operand(i, 1);
    if (divide && b == 0) b = 3;
    uint32_t fin = (rnd() & 0xF80F0000u);
    uint32_t fout;
    uint32_t r = f(a, b, fin, &fout);
    mix(r);
    mix(fout);
  }
  report(name, digest);
}

/* long multiplies (64-bit results) */
#define OP4(fn, insn)                                                                           \
  static void fn(uint32_t a, uint32_t b, uint32_t *lo, uint32_t *hi) {                          \
    uint32_t l = *lo, h = *hi;                                                                  \
    __asm volatile(insn : [l] "+&r"(l), [h] "+&r"(h) : [a] "r"(a), [b] "r"(b));                 \
    *lo = l;                                                                                    \
    *hi = h;                                                                                    \
  }
OP4(op_umull, "umull %[l], %[h], %[a], %[b]")
OP4(op_smull, "smull %[l], %[h], %[a], %[b]")
OP4(op_umlal, "umlal %[l], %[h], %[a], %[b]")
OP4(op_smlal, "smlal %[l], %[h], %[a], %[b]")
OP4(op_umaal, "umaal %[l], %[h], %[a], %[b]")
OP4(op_smlalbb, "smlalbb %[l], %[h], %[a], %[b]")
OP4(op_smlaltb, "smlaltb %[l], %[h], %[a], %[b]")
OP4(op_smlald, "smlald %[l], %[h], %[a], %[b]")
OP4(op_smlaldx, "smlaldx %[l], %[h], %[a], %[b]")
OP4(op_smlsld, "smlsld %[l], %[h], %[a], %[b]")
OP4(op_smlsldx, "smlsldx %[l], %[h], %[a], %[b]")

typedef void (*op4_t)(uint32_t, uint32_t, uint32_t *, uint32_t *);

static void run_op4(const char *name, op4_t f) {
  BEGIN(name);
  for (int i = 0; i < N + (int)(NEDGE * NEDGE); i++) {
    uint32_t a = operand(i, 0), b = operand(i, 1), lo = rnd(), hi = rnd();
    f(a, b, &lo, &hi);
    mix(lo);
    mix(hi);
  }
  report(name, digest);
}

/* ------------------------------------------------------------------ floating point */

static uint32_t fp_edges[] = {
  0x00000000, 0x80000000, 0x3F800000, 0xBF800000, 0x7F800000, 0xFF800000, 0x7FC00000, 0x7F800001,
  0xFFC00123, 0x00000001, 0x807FFFFF, 0x00800000, 0x7F7FFFFF, 0xFF7FFFFF, 0x4B000000, 0x4EFFFFFF,
  0x4F000000, 0xCF000000, 0x3EAAAAAB, 0x40490FDB, 0x34000000, 0x0C000000,
};
#define NFP (sizeof(fp_edges) / sizeof(fp_edges[0]))

static uint32_t fp_operand(int i, int which) {
  if (i < (int)(NFP * NFP)) return which ? fp_edges[i % NFP] : fp_edges[i / NFP];
  uint32_t r = rnd();
  /* bias towards nearby exponents so that arithmetic is not dominated by overflow */
  if (r & 1) r = (r & 0x807FFFFF) | ((0x70 + (r >> 26)) << 23);
  return r;
}

#define FOP(fn, insn)                                                                           \
  static uint32_t fn(uint32_t a, uint32_t b, uint32_t c, uint32_t fpscr_in, uint32_t *fpscr_out) { \
    uint32_t r, st;                                                                             \
    __asm volatile("vmsr fpscr, %[fi]\n\t"                                                      \
                   "vmov s0, %[a]\n\tvmov s1, %[b]\n\tvmov s2, %[c]\n\t" insn                   \
                   "\n\tvmov %[r], s2\n\tvmrs %[st], fpscr"                                      \
                   : [r] "=&r"(r), [st] "=&r"(st) : [a] "r"(a), [b] "r"(b), [c] "r"(c), [fi] "r"(fpscr_in) \
                   : "s0", "s1", "s2");                                                         \
    *fpscr_out = st;                                                                            \
    return r;                                                                                   \
  }
FOP(f_add, "vadd.f32 s2, s0, s1")
FOP(f_sub, "vsub.f32 s2, s0, s1")
FOP(f_mul, "vmul.f32 s2, s0, s1")
FOP(f_nmul, "vnmul.f32 s2, s0, s1")
FOP(f_div, "vdiv.f32 s2, s0, s1")
FOP(f_sqrt, "vsqrt.f32 s2, s0")
FOP(f_abs, "vabs.f32 s2, s0")
FOP(f_neg, "vneg.f32 s2, s0")
FOP(f_mla, "vmla.f32 s2, s0, s1")
FOP(f_mls, "vmls.f32 s2, s0, s1")
FOP(f_nmla, "vnmla.f32 s2, s0, s1")
FOP(f_nmls, "vnmls.f32 s2, s0, s1")
FOP(f_fma, "vfma.f32 s2, s0, s1")
FOP(f_fms, "vfms.f32 s2, s0, s1")
FOP(f_fnma, "vfnma.f32 s2, s0, s1")
FOP(f_fnms, "vfnms.f32 s2, s0, s1")
FOP(f_cmp, "vcmp.f32 s0, s1\n\tvmrs APSR_nzcv, fpscr\n\tite gt\n\tvmovgt.f32 s2, s0\n\tvmovle.f32 s2, s1")
FOP(f_cmpe, "vcmpe.f32 s0, s1\n\tvmrs APSR_nzcv, fpscr\n\tit vs\n\tvmovvs.f32 s2, s0")
FOP(f_cmpz, "vcmp.f32 s0, #0\n\tvmrs APSR_nzcv, fpscr\n\tit mi\n\tvmovmi.f32 s2, s1")
FOP(f_cvt_s32, "vcvt.s32.f32 s2, s0")
FOP(f_cvt_u32, "vcvt.u32.f32 s2, s0")
FOP(f_cvtr_s32, "vcvtr.s32.f32 s2, s0")
FOP(f_cvtr_u32, "vcvtr.u32.f32 s2, s0")
FOP(f_cvt_f_s32, "vcvt.f32.s32 s2, s0")
FOP(f_cvt_f_u32, "vcvt.f32.u32 s2, s0")
FOP(f_cvt_fix_s16, "vmov.f32 s2, s0\n\tvcvt.s16.f32 s2, s2, #7")
FOP(f_cvt_fix_u32, "vmov.f32 s2, s0\n\tvcvt.u32.f32 s2, s2, #20")
FOP(f_cvt_fix_to_f, "vmov.f32 s2, s0\n\tvcvt.f32.s32 s2, s2, #10")
FOP(f_cvt_fix_u16_to_f, "vmov.f32 s2, s0\n\tvcvt.f32.u16 s2, s2, #3")
FOP(f_cvtb_h, "vmov.f32 s2, s1\n\tvcvtb.f16.f32 s2, s0")
FOP(f_cvtt_h, "vmov.f32 s2, s1\n\tvcvtt.f16.f32 s2, s0")
FOP(f_cvtb_s, "vcvtb.f32.f16 s2, s0")
FOP(f_cvtt_s, "vcvtt.f32.f16 s2, s0")
FOP(f_movimm, "vmov.f32 s2, #-1.5\n\tvadd.f32 s2, s2, s0")

typedef uint32_t (*fop_t)(uint32_t, uint32_t, uint32_t, uint32_t, uint32_t *);

static const uint32_t fpscr_modes[] = {
  0x00000000,             /* RN */
  0x00400000,             /* RP */
  0x00800000,             /* RM */
  0x00C00000,             /* RZ */
  0x01000000,             /* FZ */
  0x02000000,             /* DN */
  0x07C00000,             /* AHP DN FZ RZ */
};

static void run_fop(const char *name, fop_t f) {
  BEGIN(name);
  for (unsigned m = 0; m < sizeof(fpscr_modes) / sizeof(fpscr_modes[0]); m++) {
    for (int i = 0; i < 150 + (int)(NFP * NFP); i++) {
      uint32_t a = fp_operand(i, 0), b = fp_operand(i, 1), c = fp_operand(i + 7, 0);
      uint32_t st;
      uint32_t r = f(a, b, c, fpscr_modes[m] | (rnd() & 0x9F), &st);
      mix(r);
      mix(st);
    }
  }
  report(name, digest);
}

/* ------------------------------------------------------------------ memory access */

static uint8_t buf[256] __attribute__((aligned(8)));

static void run_memory(void) {
  BEGIN("memory");
  for (int i = 0; i < 256; i++) buf[i] = (uint8_t)(i * 37 + 11);
  for (int i = 0; i < 200; i++) {
    uint32_t off = rnd() % 200, v1, v2, v3, v4, v5;
    uint8_t *p = buf + off;
    __asm volatile("ldr %[v1], [%[p]]\n\tldrh %[v2], [%[p], #1]\n\tldrsh %[v3], [%[p], #3]\n\t"
                   "ldrsb %[v4], [%[p], #2]\n\tldrb.w %[v5], [%[p], #5]"
                   : [v1] "=&r"(v1), [v2] "=&r"(v2), [v3] "=&r"(v3), [v4] "=&r"(v4), [v5] "=&r"(v5)
                   : [p] "r"(p) : "memory");
    mix(v1); mix(v2); mix(v3); mix(v4); mix(v5);
    uint32_t x = rnd();
    uint32_t *wp;
    __asm volatile("mov %[w], %[p]\n\tstr %[x], [%[w], #4]!\n\tstrh %[x], [%[w]], #6\n\tstrb %[x], [%[w], #-3]"
                   : [w] "=&r"(wp) : [p] "r"(buf + (off & ~7u) + 8), [x] "r"(x) : "memory");
    mix((uint32_t)((uint8_t *)wp - buf));
  }
  /* ldm/stm, ldrd/strd, push/pop */
  uint32_t a[8] __attribute__((aligned(8)));
  for (int i = 0; i < 8; i++) a[i] = rnd();
  uint32_t r0, r1, r2, r3, *pp = a;
  __asm volatile("ldmia %[p]!, {%[r0], %[r1], %[r2], %[r3]}" : [p] "+r"(pp), [r0] "=r"(r0), [r1] "=r"(r1), [r2] "=r"(r2), [r3] "=r"(r3));
  mix(r0); mix(r1); mix(r2); mix(r3); mix((uint32_t)(pp - a));
  __asm volatile("ldmdb %[p]!, {%[r0], %[r1]}" : [p] "+r"(pp), [r0] "=r"(r0), [r1] "=r"(r1));
  mix(r0); mix(r1); mix((uint32_t)(pp - a));
  __asm volatile("stmdb %[p]!, {%[r0], %[r1]}" : [p] "+r"(pp) : [r0] "r"(0x11111111), [r1] "r"(0x22222222) : "memory");
  __asm volatile("ldrd %[r0], %[r1], [%[p], #8]\n\tstrd %[r1], %[r0], [%[p]], #4" : [r0] "=&r"(r0), [r1] "=&r"(r1), [p] "+r"(pp) : : "memory");
  for (int i = 0; i < 8; i++) mix(a[i]);
  mix((uint32_t)(pp - a));
  /* exclusive monitor */
  uint32_t ex = 5, st1, st2, st3;
  __asm volatile("ldrex %[v], [%[p]]\n\tadd %[v], #1\n\tstrex %[s1], %[v], [%[p]]\n\tstrex %[s2], %[v], [%[p]]\n\t"
                 "ldrex %[v], [%[p]]\n\tclrex\n\tstrex %[s3], %[v], [%[p]]"
                 : [v] "=&r"(r0), [s1] "=&r"(st1), [s2] "=&r"(st2), [s3] "=&r"(st3) : [p] "r"(&ex) : "memory");
  mix(ex); mix(st1); mix(st2); mix(st3);
  report("memory", digest);
}

/* table branch through a dense switch (TBB/TBH) */
__attribute__((noinline)) static uint32_t sw(uint32_t x) {
  switch (x) {
    case 0: return 11; case 1: return 23; case 2: return 37; case 3: return 41; case 4: return 59;
    case 5: return 61; case 6: return 79; case 7: return 83; case 8: return 97; case 9: return 101;
    case 10: return 113; case 11: return 127; case 12: return 131; case 13: return 149; default: return 7;
  }
}

static void run_branches(void) {
  BEGIN("branches");
  for (uint32_t i = 0; i < 40; i++) mix(sw(i % 17));
  uint32_t r;
  __asm volatile("mov %[r], #0\n\tcbz %[r], 1f\n\tmov %[r], #5\n1:\tadd %[r], #1\n\tcbnz %[r], 2f\n\tmov %[r], #9\n2:" : [r] "=&r"(r));
  mix(r);
  uint32_t idx = 3, res;
  __asm volatile(
      "tbh [pc, %[i], lsl #1]\n"
      "3: .hword (4f-3b)/2, (5f-3b)/2, (6f-3b)/2, (7f-3b)/2\n"
      "4: mov %[r], #40\n b 8f\n"
      "5: mov %[r], #50\n b 8f\n"
      "6: mov %[r], #60\n b 8f\n"
      "7: mov %[r], #70\n"
      "8:"
      : [r] "=&r"(res) : [i] "r"(idx));
  mix(res);
  report("branches", digest);
}

/* ------------------------------------------------------------------ exceptions */

#define SCB_ICSR (*(volatile uint32_t *)0xE000ED04)
#define SCB_CCR (*(volatile uint32_t *)0xE000ED14)
#define SCB_SHPR2 (*(volatile uint32_t *)0xE000ED1C)
#define SCB_SHPR3 (*(volatile uint32_t *)0xE000ED20)
#define SCB_SHCSR (*(volatile uint32_t *)0xE000ED24)
#define SCB_CFSR (*(volatile uint32_t *)0xE000ED28)
#define SCB_HFSR (*(volatile uint32_t *)0xE000ED2C)
#define SYST_CSR (*(volatile uint32_t *)0xE000E010)
#define SYST_RVR (*(volatile uint32_t *)0xE000E014)
#define SYST_CVR (*(volatile uint32_t *)0xE000E018)
#define NVIC_ISER0 (*(volatile uint32_t *)0xE000E100)
#define NVIC_ISPR0 (*(volatile uint32_t *)0xE000E200)
#define NVIC_IPR0 (*(volatile uint32_t *)0xE000E400)
#define FPCCR (*(volatile uint32_t *)0xE000EF34)

static volatile uint32_t trace[32];
static volatile uint32_t ntrace;
static void t(uint32_t v) { if (ntrace < 32) trace[ntrace++] = v; }

void SVC_Handler(void) __attribute__((naked));
void SVC_Handler(void) {
  __asm volatile(
      "tst lr, #4\n\tite eq\n\tmrseq r0, msp\n\tmrsne r0, psp\n\t"
      "ldr r1, [r0, #24]\n\tldrb r1, [r1, #-2]\n\t"   /* SVC number from the instruction */
      "ldr r2, [r0]\n\tadd r2, r2, r1\n\tstr r2, [r0]\n\t" /* r0 += number */
      "bx lr");
}

static volatile float fp_in_handler;
void PendSV_Handler(void) {
  t(0x100 | (SCB_ICSR & 0x1FF));
  fp_in_handler = fp_in_handler * 3.0f + 1.0f;   /* uses the FPU: lazy stacking */
  uint32_t lr;
  __asm volatile("mov %0, lr" : "=r"(lr));
  t(lr);
}

static volatile uint32_t ticks;
void SysTick_Handler(void) {
  ticks++;
  if (ticks == 3) {
    SCB_ICSR = 1u << 28;   /* PendSV from SysTick: tail-chained after return */
    t(0x200 | ticks);
  }
}

void IRQ0_Handler(void) {
  t(0x300);
  NVIC_ISPR0 = 2;          /* IRQ1 has higher priority: preempts */
  t(0x301);
}

void IRQ1_Handler(void) { t(0x310); }

void UsageFault_Handler(void) __attribute__((naked));
void UsageFault_Handler(void) {
  __asm volatile(
      "mrs r0, msp\n\tldr r1, [r0, #24]\n\t"
      "ldrh r2, [r1]\n\tlsr r3, r2, #11\n\tcmp r3, #0x1D\n\tite cs\n\taddcs r1, #4\n\taddcc r1, #2\n\t"
      "str r1, [r0, #24]\n\t"          /* skip the faulting instruction */
      "ldr r2, =0xE000ED28\n\tldr r3, [r2]\n\tstr r3, [r2]\n\t"
      "ldr r2, =fault_cfsr\n\tstr r3, [r2]\n\tbx lr");
}
volatile uint32_t fault_cfsr;

static void run_exceptions(void) {
  BEGIN("exceptions");
  /* SVC: r0 += svc number */
  register uint32_t r0 __asm("r0") = 100;
  __asm volatile("svc #23" : "+r"(r0));
  mix(r0);
  /* PendSV with lazy FP stacking: thread FP registers survive */
  SCB_SHPR3 = (0xE0u << 16) | (0x40u << 24);   /* PendSV lowest, SysTick 0x40 */
  float keep = 1.25f;
  __asm volatile("vmov.f32 s16, #1.25" ::: "s16");
  fp_in_handler = 2.0f;
  SCB_ICSR = 1u << 28;
  __asm volatile("dsb\n isb");
  float after;
  __asm volatile("vmov %0, s16" : "=r"(after));
  mix(*(uint32_t *)&after == *(uint32_t *)&keep);
  mix(*(uint32_t *)&fp_in_handler);
  mix(FPCCR);
  /* SysTick: three interrupts, then PendSV pended from the handler */
  ticks = 0;
  SYST_RVR = 999;
  SYST_CVR = 0;
  SYST_CSR = 7;
  while (ticks < 5) {}
  SYST_CSR = 0;
  mix(ticks);
  /* NVIC preemption: IRQ0 low priority, IRQ1 high */
  NVIC_IPR0 = (0x80u << 0) | (0x20u << 8);
  NVIC_ISER0 = 3;
  NVIC_ISPR0 = 1;
  __asm volatile("dsb\n isb");
  /* BASEPRI masks IRQ0 */
  __asm volatile("msr basepri, %0" ::"r"(0x80));
  NVIC_ISPR0 = 1;
  __asm volatile("dsb\n isb");
  t(0x400);
  __asm volatile("msr basepri, %0" ::"r"(0));
  __asm volatile("dsb\n isb");
  /* PRIMASK */
  __asm volatile("cpsid i");
  NVIC_ISPR0 = 2;
  t(0x500 | ((NVIC_ISPR0 >> 1) & 1));
  __asm volatile("cpsie i");
  __asm volatile("dsb\n isb");
  /* usage faults, handled and skipped */
  SCB_SHCSR |= 1u << 18;
  SCB_CCR |= (1u << 4) | (1u << 3);   /* DIV_0_TRP, UNALIGN_TRP */
  volatile uint32_t z = 0, q = 7, v;
  v = q / z;
  mix(fault_cfsr);
  fault_cfsr = 0;
  v = *(volatile uint32_t *)((uint8_t *)buf + 1);
  mix(fault_cfsr);
  fault_cfsr = 0;
  __asm volatile("udf #0");
  mix(fault_cfsr);
  SCB_CCR &= ~((1u << 4) | (1u << 3));
  (void)v;
  for (uint32_t i = 0; i < ntrace; i++) mix(trace[i]);
  mix(ntrace);
  report("exceptions", digest);
}

/* ------------------------------------------------------------------ main */

int main(void) {
  puts_("isa-test begin\n");
  run_op2f("adds", op_adds, 0);
  run_op2f("adcs", op_adcs, 0);
  run_op2f("subs", op_subs, 0);
  run_op2f("sbcs", op_sbcs, 0);
  run_op2f("rsbs", op_rsbs, 0);
  run_op2f("cmp", op_cmp, 0);
  run_op2f("cmn", op_cmn, 0);
  run_op2f("ands_lsl", op_ands_lsl, 0);
  run_op2f("orrs_lsr", op_orrs_lsr, 0);
  run_op2f("eors_asr", op_eors_asr, 0);
  run_op2f("bics_ror", op_bics_ror, 0);
  run_op2f("orns_rrx", op_orns_rrx, 0);
  run_op2f("mvns_lsl", op_mvns_lsl, 0);
  run_op2f("tst", op_tst, 0);
  run_op2f("teq", op_teq, 0);
  run_op2f("adds_imm", op_adds_imm, 0);
  run_op2f("subs_imm", op_subs_imm, 0);
  run_op2f("ands_imm", op_ands_imm, 0);
  run_op2f("lsls", op_lsls, 0);
  run_op2f("lsrs", op_lsrs, 0);
  run_op2f("asrs", op_asrs, 0);
  run_op2f("rors", op_rors, 0);
  run_op2f("lsls_imm", op_lsls_imm, 0);
  run_op2f("lsrs_imm", op_lsrs_imm, 0);
  run_op2f("asrs_imm", op_asrs_imm, 0);
  run_op2f("rrxs", op_rrxs, 0);
  run_op2f("muls", op_muls, 0);
  run_op2f("mla", op_mla, 0);
  run_op2f("mls", op_mls, 0);
  run_op2f("sdiv", op_sdiv, 0);
  run_op2f("udiv", op_udiv, 0);
  run_op2f("qadd", op_qadd, 0);
  run_op2f("qsub", op_qsub, 0);
  run_op2f("qdadd", op_qdadd, 0);
  run_op2f("qdsub", op_qdsub, 0);
  run_op2f("ssat", op_ssat, 0);
  run_op2f("ssat32", op_ssat2, 0);
  run_op2f("usat", op_usat, 0);
  run_op2f("usat0", op_usat0, 0);
  run_op2f("ssat16", op_ssat16, 0);
  run_op2f("usat16", op_usat16, 0);
  run_op2f("sadd16", op_sadd16, 0);
  run_op2f("ssub16", op_ssub16, 0);
  run_op2f("sasx", op_sasx, 0);
  run_op2f("ssax", op_ssax, 0);
  run_op2f("sadd8", op_sadd8, 0);
  run_op2f("ssub8", op_ssub8, 0);
  run_op2f("uadd16", op_uadd16, 0);
  run_op2f("usub16", op_usub16, 0);
  run_op2f("uasx", op_uasx, 0);
  run_op2f("usax", op_usax, 0);
  run_op2f("uadd8", op_uadd8, 0);
  run_op2f("usub8", op_usub8, 0);
  run_op2f("qadd16", op_qadd16, 0);
  run_op2f("qsub16", op_qsub16, 0);
  run_op2f("qasx", op_qasx, 0);
  run_op2f("qsax", op_qsax, 0);
  run_op2f("qadd8", op_qadd8, 0);
  run_op2f("qsub8", op_qsub8, 0);
  run_op2f("uqadd16", op_uqadd16, 0);
  run_op2f("uqsub16", op_uqsub16, 0);
  run_op2f("uqasx", op_uqasx, 0);
  run_op2f("uqsax", op_uqsax, 0);
  run_op2f("uqadd8", op_uqadd8, 0);
  run_op2f("uqsub8", op_uqsub8, 0);
  run_op2f("shadd16", op_shadd16, 0);
  run_op2f("shsub16", op_shsub16, 0);
  run_op2f("shasx", op_shasx, 0);
  run_op2f("shsax", op_shsax, 0);
  run_op2f("shadd8", op_shadd8, 0);
  run_op2f("shsub8", op_shsub8, 0);
  run_op2f("uhadd16", op_uhadd16, 0);
  run_op2f("uhsub16", op_uhsub16, 0);
  run_op2f("uhasx", op_uhasx, 0);
  run_op2f("uhsax", op_uhsax, 0);
  run_op2f("uhadd8", op_uhadd8, 0);
  run_op2f("uhsub8", op_uhsub8, 0);
  run_op2f("sel", op_sel, 0);
  run_op2f("smulbb", op_smulbb, 0);
  run_op2f("smultb", op_smultb, 0);
  run_op2f("smulbt", op_smulbt, 0);
  run_op2f("smultt", op_smultt, 0);
  run_op2f("smlabb", op_smlabb, 0);
  run_op2f("smlatt", op_smlatt, 0);
  run_op2f("smulwb", op_smulwb, 0);
  run_op2f("smulwt", op_smulwt, 0);
  run_op2f("smlawb", op_smlawb, 0);
  run_op2f("smlawt", op_smlawt, 0);
  run_op2f("smuad", op_smuad, 0);
  run_op2f("smuadx", op_smuadx, 0);
  run_op2f("smusd", op_smusd, 0);
  run_op2f("smusdx", op_smusdx, 0);
  run_op2f("smlad", op_smlad, 0);
  run_op2f("smladx", op_smladx, 0);
  run_op2f("smlsd", op_smlsd, 0);
  run_op2f("smlsdx", op_smlsdx, 0);
  run_op2f("smmul", op_smmul, 0);
  run_op2f("smmulr", op_smmulr, 0);
  run_op2f("smmla", op_smmla, 0);
  run_op2f("smmlar", op_smmlar, 0);
  run_op2f("smmls", op_smmls, 0);
  run_op2f("smmlsr", op_smmlsr, 0);
  run_op2f("usad8", op_usad8, 0);
  run_op2f("usada8", op_usada8, 0);
  run_op2f("clz", op_clz, 0);
  run_op2f("rbit", op_rbit, 0);
  run_op2f("rev", op_rev, 0);
  run_op2f("rev16", op_rev16, 0);
  run_op2f("revsh", op_revsh, 0);
  run_op2f("ubfx", op_ubfx, 0);
  run_op2f("sbfx", op_sbfx, 0);
  run_op2f("bfi", op_bfi, 0);
  run_op2f("bfc", op_bfc, 0);
  run_op2f("uxtab", op_uxtab, 0);
  run_op2f("sxtab", op_sxtab, 0);
  run_op2f("uxtah", op_uxtah, 0);
  run_op2f("sxtah", op_sxtah, 0);
  run_op2f("uxtab16", op_uxtab16, 0);
  run_op2f("sxtab16", op_sxtab16, 0);
  run_op2f("uxtb16", op_uxtb16, 0);
  run_op2f("sxtb16", op_sxtb16, 0);
  run_op2f("pkhbt", op_pkhbt, 0);
  run_op2f("pkhtb", op_pkhtb, 0);
  run_op2f("movw_movt", op_movw_movt, 0);
  run_op2f("it", op_it, 0);
  run_op2f("it16", op_it16, 0);
  run_op4("umull", op_umull);
  run_op4("smull", op_smull);
  run_op4("umlal", op_umlal);
  run_op4("smlal", op_smlal);
  run_op4("umaal", op_umaal);
  run_op4("smlalbb", op_smlalbb);
  run_op4("smlaltb", op_smlaltb);
  run_op4("smlald", op_smlald);
  run_op4("smlaldx", op_smlaldx);
  run_op4("smlsld", op_smlsld);
  run_op4("smlsldx", op_smlsldx);
  run_fop("vadd", f_add);
  run_fop("vsub", f_sub);
  run_fop("vmul", f_mul);
  run_fop("vnmul", f_nmul);
  run_fop("vdiv", f_div);
  run_fop("vsqrt", f_sqrt);
  run_fop("vabs", f_abs);
  run_fop("vneg", f_neg);
  run_fop("vmla", f_mla);
  run_fop("vmls", f_mls);
  run_fop("vnmla", f_nmla);
  run_fop("vnmls", f_nmls);
  run_fop("vfma", f_fma);
  run_fop("vfms", f_fms);
  run_fop("vfnma", f_fnma);
  run_fop("vfnms", f_fnms);
  run_fop("vcmp", f_cmp);
  run_fop("vcmpe", f_cmpe);
  run_fop("vcmpz", f_cmpz);
  run_fop("vcvt_s32", f_cvt_s32);
  run_fop("vcvt_u32", f_cvt_u32);
  run_fop("vcvtr_s32", f_cvtr_s32);
  run_fop("vcvtr_u32", f_cvtr_u32);
  run_fop("vcvt_f_s32", f_cvt_f_s32);
  run_fop("vcvt_f_u32", f_cvt_f_u32);
  run_fop("vcvt_fix_s16", f_cvt_fix_s16);
  run_fop("vcvt_fix_u32", f_cvt_fix_u32);
  run_fop("vcvt_fix_to_f", f_cvt_fix_to_f);
  run_fop("vcvt_fix_u16_to_f", f_cvt_fix_u16_to_f);
  run_fop("vcvtb_f16", f_cvtb_h);
  run_fop("vcvtt_f16", f_cvtt_h);
  run_fop("vcvtb_f32", f_cvtb_s);
  run_fop("vcvtt_f32", f_cvtt_s);
  run_fop("vmov_imm", f_movimm);
  run_memory();
  run_branches();
  run_exceptions();
  puts_("isa-test end\n");
  return 0;
}
