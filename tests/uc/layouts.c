#include <stdio.h>
#include <stddef.h>
#include <stdint.h>

struct S0 {
    long long f0;
    int f1[4];
    long f2;
};
struct S1 {
    long long f0;
    int32_t f1;
    struct S0 f2;
};
struct S2 {
    long int f0;
    struct S1 f1;
    int g0, g1[2];
};
struct S3 {
    struct S0 f0;
    char * f1;
    long int f2[2];
    struct S1 *f3;
    long long f4[3];
    struct S2 f5[1];
};
struct S4 {
    struct S3 f0;
    struct S0 f1[2];
    struct S0 f2;
    char f3[1];
    int32_t f4[4];
    long f5[7];
    struct S3 f6[2];
    signed char f7;
    int g0, g1[2];
};
struct S5 {
    struct S1 *f0;
    struct S3 f1;
    double f2[5];
    struct S2 f3;
    double f4[7];
};
struct S6 {
    double f0;
    struct S3 f1[1];
    int32_t f2;
    struct S0 *f3;
    long f4[5];
    int32_t f5;
    struct S4 *f6;
};
struct S7 {
    char * f0;
    short int f1;
    signed char f2[5];
    struct S5 f3[3];
    int64_t f4;
    struct S6 f5[3];
    int64_t f6[7];
};
struct S8 {
    struct S7 f0;
    struct S7 f1;
    struct S1 f2[3];
    struct S5 f3;
    struct S0 *f4;
    struct S2 *f5;
    char * f6;
    short int f7;
    char g0, g1[2];
};
struct S9 {
    long int f0;
    double f1;
    int f2;
    long int f3;
    char f4;
};
struct S10 {
    struct S0 f0;
    struct S7 f1;
    struct S2 *f2;
    struct S9 *f3;
    int f4;
    long f5[4];
};
struct S11 {
    signed char f0;
    struct S2 f1[2];
    struct S11 *f2;
    long long g0, g1[2];
};
struct S12 {
    struct S0 *f0;
    char f1;
    double f2[1];
    struct S7 f3;
    struct S6 f4;
    long int f5;
    signed char f6;
    struct S9 *f7;
    long long g0, g1[2];
};
struct S13 {
    long f0;
    long int f1[2];
    struct S7 f2[2];
    short int f3;
    struct S3 f4;
    struct S3 f5;
    int f6;
};
struct S14 {
    signed char f0[6];
    long int f1;
    struct S13 f2;
    char g0, g1[2];
};
struct S15 {
    int f0;
    struct S0 f1;
    long int f2;
    int f3[1];
    double f4;
    struct S9 *f5;
};
struct S16 {
    struct S11 f0;
    short int f1;
    int f2;
    struct S10 f3;
};
struct S17 {
    int32_t f0;
};
struct S18 {
    struct S16 f0;
    struct S4 f1;
    struct S10 f2[3];
    struct S1 f3[1];
    struct S17 f4;
    struct S4 f5[3];
};
struct S19 {
    struct S14 f0;
    char * f1;
    short int f2;
    long f3;
    struct S6 *f4;
};
struct S20 {
    short f0;
    double f1;
    long long g0, g1[2];
};
struct S21 {
    short int f0;
    signed char f1;
    int f2;
    signed char f3;
    int f4[1];
    long long g0, g1[2];
};
struct S22 {
    struct S3 f0;
    int64_t f1;
};
struct S23 {
    struct S14 f0;
    char f1;
};
struct S24 {
    int32_t f0[5];
    struct S19 *f1;
    struct S17 f2;
    long f3;
    struct S21 f4;
    struct S20 f5[3];
    struct S7 f6;
};
struct S25 {
    char * f0;
    short g0, g1[2];
};
struct S26 {
    long int f0;
    int f1;
    struct S5 *f2;
    long f3;
    struct S23 f4;
    struct S15 f5[1];
    long int f6;
    long long f7[2];
};
struct S27 {
    struct S26 f0;
    struct S22 f1;
    long long f2;
    int32_t f3;
    struct S23 f4[2];
    char * f5;
    struct S9 f6[3];
    long long g0, g1[2];
};
struct S28 {
    char f0;
    long f1;
};
struct S29 {
    struct S25 *f0;
    struct S27 f1;
    char g0, g1[2];
};
struct S30 {
    int32_t f0[2];
    long int f1[4];
    struct S24 f2[2];
    double f3;
    long long f4;
    long long f5;
};
struct S31 {
    long int f0;
    short f1;
};
struct S32 {
    short f0[5];
    long long f1;
    struct S22 f2;
    struct S31 *f3;
    long long g0, g1[2];
};
struct S33 {
    struct S30 *f0;
    struct S20 f1[3];
    double f2;
    int64_t f3;
    int64_t f4;
    struct S16 f5;
};
struct S34 {
    struct S9 f0;
    struct S2 f1;
    char f2;
    struct S11 *f3;
    long int f4;
    struct S27 f5;
    struct S13 f6;
};
struct S35 {
    struct S26 f0[2];
    long long g0, g1[2];
};
struct S36 {
    struct S13 f0[1];
    int f1;
    struct S0 f2[1];
};
struct S37 {
    long f0[7];
    int32_t f1[6];
    char * f2;
    struct S6 *f3;
    struct S30 f4;
    struct S1 f5[3];
    char f6;
    double f7[2];
};
struct S38 {
    struct S20 f0[2];
    int32_t f1;
    double f2;
    struct S19 f3;
    struct S12 f4[1];
    long long g0, g1[2];
};
struct S39 {
    int64_t f0[7];
    struct S37 *f1;
    long f2;
    int32_t f3;
    long long f4;
    short f5[3];
    char f6;
};
struct S40 {
    struct S0 f0;
    struct S2 *f1;
    short int f2;
    struct S7 f3[1];
    int g0, g1[2];
};
struct S41 {
    struct S8 f0;
    long int f1;
};
struct S42 {
    struct S7 f0;
    long long f1;
    short f2;
};
struct S43 {
    struct S13 f0;
    long long g0, g1[2];
};
struct S44 {
    long f0[4];
    struct S33 f1;
    long f2;
    struct S7 f3[3];
    struct S14 f4;
    double f5;
    struct S13 f6;
    struct S43 *f7;
    int g0, g1[2];
};
struct S45 {
    long f0;
    struct S27 *f1;
    struct S19 *f2;
    struct S39 f3[1];
    struct S14 f4;
    short f5;
    struct S9 *f6;
};
struct S46 {
    struct S10 f0[1];
    struct S3 f1;
    int64_t f2;
    struct S9 f3[1];
    struct S31 *f4;
    struct S34 f5;
    short int f6;
    short g0, g1[2];
};
struct S47 {
    short f0[7];
    struct S23 f1[3];
    int g0, g1[2];
};
struct S48 {
    struct S7 f0[1];
    struct S41 *f1;
};
struct S49 {
    struct S9 f0;
    long long f1;
    char f2[1];
};
struct S50 {
    int f0;
    short f1;
    struct S1 f2[1];
    struct S9 f3;
    signed char f4;
    long f5;
    struct S4 f6[3];
    int g0, g1[2];
};
struct S51 {
    signed char f0;
    int f1[2];
};
struct S52 {
    long long f0;
};
struct S53 {
    signed char f0;
    struct S12 f1;
    int64_t f2;
};
struct S54 {
    struct S7 *f0;
    int32_t f1;
    int32_t f2[1];
    struct S33 f3;
    long int f4;
};
struct S55 {
    struct S2 f0;
    double f1[5];
    struct S28 f2[1];
    struct S44 f3;
    char * f4;
    struct S30 f5;
    struct S25 f6;
    double f7;
};
struct S56 {
    struct S41 f0;
    long f1;
    struct S31 *f2;
    struct S22 f3;
};
struct S57 {
    int64_t f0;
    short f1;
    struct S53 f2;
    long f3;
    struct S40 f4[3];
};
struct S58 {
    char f0;
    short f1;
    struct S15 f2[3];
    struct S29 f3;
    struct S6 f4[1];
    int f5[3];
    struct S15 *f6;
    int g0, g1[2];
};
struct S59 {
    struct S12 f0;
    char g0, g1[2];
};
struct S60 {
    struct S53 f0[2];
};
struct S61 {
    int32_t f0;
};
struct S62 {
    int64_t f0;
    int f1;
};
struct S63 {
    int64_t f0;
    struct S26 f1[2];
    short f2[5];
    int32_t f3;
    struct S47 f4;
    struct S10 f5[1];
    char g0, g1[2];
};
struct S64 {
    int64_t f0;
    int f1;
    struct S61 f2[1];
    struct S8 f3;
};
struct S65 {
    int f0;
    short f1[2];
    long f2;
};
struct S66 {
    struct S1 f0[2];
    long int f1;
    int f2[4];
};
struct S67 {
    double f0[2];
};
struct S68 {
    struct S34 *f0;
    char f1;
    short f2;
    char f3[5];
    short int f4[3];
};
struct S69 {
    struct S30 *f0;
    struct S5 f1;
    short int f2;
};
struct S70 {
    struct S51 f0[2];
    struct S33 f1[1];
    short g0, g1[2];
};
struct S71 {
    signed char f0;
    char * f1;
    struct S51 *f2;
    struct S60 f3[2];
};
struct S72 {
    struct S28 *f0;
};
struct S73 {
    short int f0[2];
    struct S3 f1[3];
    signed char f2;
};
struct S74 {
    struct S47 f0[3];
};
struct S75 {
    char f0;
    signed char f1;
    struct S27 f2;
    struct S7 *f3;
    struct S50 f4;
    struct S71 f5[1];
};
struct S76 {
    struct S74 f0[2];
    char * f1;
    char f2[4];
    double f3;
};
struct S77 {
    long int f0;
    struct S58 *f1;
    int64_t f2[1];
    char f3;
    struct S53 f4[1];
    double f5[5];
    struct S1 f6[3];
    int f7;
};
struct S78 {
    signed char f0;
    struct S21 f1[2];
    char * f2;
    struct S8 *f3;
    struct S2 f4[2];
    char f5;
    int32_t f6;
};
struct S79 {
    struct S69 *f0;
    long long f1[7];
    short g0, g1[2];
};
struct S80 {
    struct S30 f0[3];
    struct S8 f1[2];
    char g0, g1[2];
};
struct S81 {
    short f0;
    long long f1;
    struct S8 *f2;
    int64_t f3[7];
    signed char f4;
    struct S38 f5;
    char * f6;
    struct S10 f7;
};
struct S82 {
    struct S10 f0;
    struct S9 f1[2];
    short int f2;
};
struct S83 {
    long int f0;
    struct S37 f1[3];
    int32_t f2[7];
};
struct S84 {
    struct S68 f0;
    long f1;
    char * f2;
    double f3;
    short int f4;
    struct S83 f5;
    struct S56 f6[1];
    struct S78 f7[3];
    short g0, g1[2];
};
struct S85 {
    struct S81 *f0;
    struct S45 *f1;
};
struct S86 {
    double f0;
    double f1;
    int32_t f2;
    struct S64 f3[2];
    struct S7 f4;
    struct S73 f5;
    int g0, g1[2];
};
struct S87 {
    int f0;
    struct S84 *f1;
    struct S86 *f2;
    struct S26 f3[3];
    short int f4;
};
struct S88 {
    long int f0;
    int f1[7];
};
struct S89 {
    struct S20 f0[2];
    struct S79 f1;
    struct S16 f2[3];
    struct S23 f3;
    struct S65 f4[3];
    short int f5;
    long f6[3];
};
struct S90 {
    struct S50 f0[1];
    struct S3 f1;
    struct S88 *f2;
    signed char f3;
    struct S71 f4;
    struct S31 f5;
};
struct S91 {
    struct S53 f0[2];
};
struct S92 {
    struct S31 f0;
    short f1;
    char f2;
    char f3[2];
    short g0, g1[2];
};
struct S93 {
    signed char f0;
    struct S38 f1;
    struct S69 f2;
    struct S64 f3;
    struct S92 f4;
    int g0, g1[2];
};
struct S94 {
    int64_t f0;
    struct S21 f1[2];
    struct S32 f2;
    double f3;
    struct S69 f4;
    struct S42 f5[3];
    struct S74 f6;
    int64_t f7;
};
struct S95 {
    int64_t f0;
    struct S13 f1;
    struct S84 *f2;
    short g0, g1[2];
};
struct S96 {
    struct S33 f0;
    long f1[1];
    struct S60 f2;
    struct S77 f3;
};
struct S97 {
    struct S26 f0;
    double f1[7];
    struct S35 f2;
    struct S96 f3;
    int f4;
    struct S55 f5[1];
    short int f6;
};
struct S98 {
    struct S74 f0;
    long f1;
    struct S11 f2[1];
    struct S78 f3[1];
    struct S87 f4;
    short int f5;
};
struct S99 {
    long f0;
    double f1;
    struct S76 f2[2];
    long f3;
    long long g0, g1[2];
};
struct S100 {
    struct S42 f0;
    int g0, g1[2];
};
struct S101 {
    struct S96 f0[1];
    int f1;
    int32_t f2;
};
struct S102 {
    struct S10 f0[3];
    struct S66 f1;
    struct S37 *f2;
    char f3;
};
struct S103 {
    struct S46 f0[1];
};
struct S104 {
    signed char f0;
    struct S76 f1[3];
    struct S56 f2[1];
    struct S74 *f3;
    double f4[7];
    struct S98 *f5;
    int32_t f6;
    struct S83 f7[2];
};
struct S105 {
    struct S35 f0;
    signed char f1[6];
    struct S2 f2;
    struct S12 f3;
    signed char f4[5];
    struct S94 *f5;
    short g0, g1[2];
};
struct S106 {
    char * f0;
    long f1;
    struct S47 *f2;
    short int f3;
    int32_t f4;
    long long f5;
    struct S98 f6[3];
    struct S72 *f7;
    short g0, g1[2];
};
struct S107 {
    struct S22 *f0;
    int64_t f1;
    struct S35 f2[1];
    short int f3;
    struct S60 f4[2];
    struct S101 f5[3];
    struct S38 f6;
};
struct S108 {
    long long f0;
};
struct S109 {
    char f0[2];
    struct S77 f1;
    struct S0 f2;
    short g0, g1[2];
};
struct S110 {
    struct S48 f0;
    char f1;
    struct S49 f2;
    struct S60 f3;
};
struct S111 {
    struct S40 f0;
    struct S68 f1[2];
    signed char f2[7];
    struct S86 f3[1];
    char f4;
    long long f5[7];
    struct S19 f6;
    int64_t f7[1];
};
struct S112 {
    char f0;
    struct S52 f1[1];
    short int f2;
    short int f3[4];
    struct S95 *f4;
    struct S66 f5[1];
    struct S80 f6;
    long f7;
};
struct S113 {
    struct S104 f0;
    struct S18 *f1;
    struct S69 *f2;
    char f3;
};
struct S114 {
    struct S4 f0;
    long int f1;
    char * f2;
    signed char f3;
    struct S111 f4;
    char * f5;
    short f6[1];
    int f7;
};
struct S115 {
    long f0;
    struct S106 f1[1];
    long int f2;
    long int f3;
    struct S105 f4[1];
    struct S102 f5;
    struct S6 f6;
    struct S88 f7;
};
struct S116 {
    struct S84 f0[2];
    char * f1;
    struct S44 f2;
};
struct S117 {
    struct S79 *f0;
    long f1;
    int f2;
    struct S34 f3;
    struct S97 f4;
    long f5;
};
struct S118 {
    struct S47 f0;
    struct S104 f1;
};
struct S119 {
    struct S63 f0;
    struct S75 f1;
};
struct S120 {
    short f0;
    struct S41 f1;
};
struct S121 {
    char * f0;
    struct S69 *f1;
    long f2;
};
struct S122 {
    char * f0;
    int32_t f1;
    int32_t f2[7];
    struct S84 *f3;
    struct S62 *f4;
    char f5;
    int f6;
    int f7;
    char g0, g1[2];
};
struct S123 {
    struct S20 f0;
    int32_t f1;
    short int f2;
    int g0, g1[2];
};
struct S124 {
    short int f0;
};
struct S125 {
    double f0;
    int64_t f1;
    char f2[6];
    long int f3;
    struct S90 f4;
    struct S56 f5[2];
};
struct S126 {
    long f0;
    int32_t f1;
    struct S69 f2;
    struct S5 f3;
    struct S53 f4;
    long long g0, g1[2];
};
struct S127 {
    struct S20 *f0;
};
struct S128 {
    struct S4 f0;
    long int f1[6];
    struct S46 f2[2];
    signed char f3;
    char * f4;
    struct S91 f5;
};
struct S129 {
    char * f0;
    struct S116 f1[1];
};
struct S130 {
    struct S107 *f0;
    struct S115 f1[3];
    short f2;
    struct S52 f3;
    signed char f4;
    int g0, g1[2];
};
struct S131 {
    long long f0;
    int f1;
    short f2;
    short f3;
    int32_t f4;
    short int f5[5];
    int g0, g1[2];
};
struct S132 {
    struct S0 f0[1];
    long int f1;
    struct S130 f2[2];
    struct S21 f3[3];
    int32_t f4[7];
    int f5;
    char * f6;
    long long f7;
};
struct S133 {
    int f0;
    short int f1;
    char * f2;
    struct S35 f3[2];
    struct S68 f4;
    long int f5;
    short int f6;
    long f7;
};
struct S134 {
    signed char f0;
    int f1;
    short int f2;
    struct S48 f3[1];
    struct S84 f4;
    short g0, g1[2];
};
struct S135 {
    int f0;
    long long f1;
    int32_t f2;
    struct S33 f3;
    long long g0, g1[2];
};
struct S136 {
    struct S13 f0;
    int f1[2];
    struct S22 f2[2];
    int64_t f3;
    struct S11 f4;
    long f5;
    char f6[1];
    struct S122 f7[2];
    char g0, g1[2];
};
struct S137 {
    long long f0;
    struct S64 f1[2];
    int64_t f2;
    struct S54 f3[3];
};
struct S138 {
    short f0;
    long int f1;
    short f2;
    long int f3;
    struct S125 f4;
    short f5;
    struct S12 f6;
    short int f7;
    char g0, g1[2];
};
struct S139 {
    struct S63 f0;
    short int f1[2];
    long f2;
    long int f3;
    struct S98 *f4;
    struct S123 f5;
    long int f6;
    char g0, g1[2];
};
struct S140 {
    struct S120 f0;
    long f1;
    int f2;
    struct S105 *f3;
    long int f4;
    long int f5;
};
struct S141 {
    struct S17 *f0;
};
struct S142 {
    int64_t f0;
    struct S44 f1;
    struct S93 f2;
    signed char f3;
    struct S90 *f4;
    struct S21 f5;
    struct S127 *f6;
    char * f7;
};
struct S143 {
    struct S47 f0[2];
    struct S13 f1;
    long int f2;
    struct S70 f3;
    struct S55 f4[2];
    long long f5;
    int64_t f6[1];
    int32_t f7;
};
struct S144 {
    short f0[2];
    long long f1;
    char f2;
    struct S31 f3;
    struct S80 f4[3];
};
struct S145 {
    struct S90 f0;
    struct S70 *f1;
    int f2;
    struct S19 f3;
    struct S53 f4;
    struct S118 f5;
    struct S84 f6;
    char g0, g1[2];
};
struct S146 {
    struct S4 *f0;
};
struct S147 {
    struct S134 f0;
    char f1;
    long f2;
    struct S93 f3[1];
    long int f4;
    struct S67 f5;
    struct S141 f6[1];
    struct S76 f7[2];
    long long g0, g1[2];
};
struct S148 {
    struct S69 f0;
    int f1[1];
    short int f2;
    struct S9 f3[3];
};
struct S149 {
    struct S42 *f0;
    struct S80 f1;
    struct S13 f2;
    struct S18 f3[2];
    struct S123 f4;
    struct S36 *f5;
    int g0, g1[2];
};
struct S150 {
    struct S110 f0[2];
    struct S143 f1[1];
    int f2[1];
    struct S60 f3;
    struct S101 f4;
};
struct S151 {
    struct S75 f0[3];
    double f1[1];
    short f2;
    int f3;
    struct S99 f4;
};
struct S152 {
    struct S48 f0[3];
    int64_t f1;
    struct S2 f2[1];
    struct S44 f3;
    int f4[7];
    struct S27 f5;
    struct S11 *f6;
};
struct S153 {
    struct S9 f0;
    signed char f1;
    struct S81 f2[2];
    struct S55 f3[3];
};
struct S154 {
    struct S28 f0;
    short g0, g1[2];
};
struct S155 {
    struct S44 f0;
    struct S39 f1[3];
    short g0, g1[2];
};
struct S156 {
    int64_t f0;
    long int f1;
    int64_t f2[2];
    char g0, g1[2];
};
struct S157 {
    short int f0;
    char * f1;
    int f2[1];
    long f3;
    struct S14 *f4;
    short int f5;
    struct S95 f6[1];
};
struct S158 {
    double f0;
    struct S5 f1[1];
    long long g0, g1[2];
};
struct S159 {
    short f0;
    struct S21 *f1;
    struct S122 f2;
    struct S32 f3;
    struct S69 f4;
    struct S28 f5[1];
    signed char f6;
};

int main() {
    printf("S0: %zu %zu %zu %zu\n", sizeof(struct S0), offsetof(struct S0, f0), offsetof(struct S0, f1), offsetof(struct S0, f2));
    printf("S1: %zu %zu %zu %zu\n", sizeof(struct S1), offsetof(struct S1, f0), offsetof(struct S1, f1), offsetof(struct S1, f2));
    printf("S2: %zu %zu %zu %zu %zu\n", sizeof(struct S2), offsetof(struct S2, f0), offsetof(struct S2, f1), offsetof(struct S2, g0), offsetof(struct S2, g1));
    printf("S3: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S3), offsetof(struct S3, f0), offsetof(struct S3, f1), offsetof(struct S3, f2), offsetof(struct S3, f3), offsetof(struct S3, f4), offsetof(struct S3, f5));
    printf("S4: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S4), offsetof(struct S4, f0), offsetof(struct S4, f1), offsetof(struct S4, f2), offsetof(struct S4, f3), offsetof(struct S4, f4), offsetof(struct S4, f5), offsetof(struct S4, f6), offsetof(struct S4, f7), offsetof(struct S4, g0), offsetof(struct S4, g1));
    printf("S5: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S5), offsetof(struct S5, f0), offsetof(struct S5, f1), offsetof(struct S5, f2), offsetof(struct S5, f3), offsetof(struct S5, f4));
    printf("S6: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S6), offsetof(struct S6, f0), offsetof(struct S6, f1), offsetof(struct S6, f2), offsetof(struct S6, f3), offsetof(struct S6, f4), offsetof(struct S6, f5), offsetof(struct S6, f6));
    printf("S7: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S7), offsetof(struct S7, f0), offsetof(struct S7, f1), offsetof(struct S7, f2), offsetof(struct S7, f3), offsetof(struct S7, f4), offsetof(struct S7, f5), offsetof(struct S7, f6));
    printf("S8: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S8), offsetof(struct S8, f0), offsetof(struct S8, f1), offsetof(struct S8, f2), offsetof(struct S8, f3), offsetof(struct S8, f4), offsetof(struct S8, f5), offsetof(struct S8, f6), offsetof(struct S8, f7), offsetof(struct S8, g0), offsetof(struct S8, g1));
    printf("S9: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S9), offsetof(struct S9, f0), offsetof(struct S9, f1), offsetof(struct S9, f2), offsetof(struct S9, f3), offsetof(struct S9, f4));
    printf("S10: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S10), offsetof(struct S10, f0), offsetof(struct S10, f1), offsetof(struct S10, f2), offsetof(struct S10, f3), offsetof(struct S10, f4), offsetof(struct S10, f5));
    printf("S11: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S11), offsetof(struct S11, f0), offsetof(struct S11, f1), offsetof(struct S11, f2), offsetof(struct S11, g0), offsetof(struct S11, g1));
    printf("S12: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S12), offsetof(struct S12, f0), offsetof(struct S12, f1), offsetof(struct S12, f2), offsetof(struct S12, f3), offsetof(struct S12, f4), offsetof(struct S12, f5), offsetof(struct S12, f6), offsetof(struct S12, f7), offsetof(struct S12, g0), offsetof(struct S12, g1));
    printf("S13: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S13), offsetof(struct S13, f0), offsetof(struct S13, f1), offsetof(struct S13, f2), offsetof(struct S13, f3), offsetof(struct S13, f4), offsetof(struct S13, f5), offsetof(struct S13, f6));
    printf("S14: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S14), offsetof(struct S14, f0), offsetof(struct S14, f1), offsetof(struct S14, f2), offsetof(struct S14, g0), offsetof(struct S14, g1));
    printf("S15: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S15), offsetof(struct S15, f0), offsetof(struct S15, f1), offsetof(struct S15, f2), offsetof(struct S15, f3), offsetof(struct S15, f4), offsetof(struct S15, f5));
    printf("S16: %zu %zu %zu %zu %zu\n", sizeof(struct S16), offsetof(struct S16, f0), offsetof(struct S16, f1), offsetof(struct S16, f2), offsetof(struct S16, f3));
    printf("S17: %zu %zu\n", sizeof(struct S17), offsetof(struct S17, f0));
    printf("S18: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S18), offsetof(struct S18, f0), offsetof(struct S18, f1), offsetof(struct S18, f2), offsetof(struct S18, f3), offsetof(struct S18, f4), offsetof(struct S18, f5));
    printf("S19: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S19), offsetof(struct S19, f0), offsetof(struct S19, f1), offsetof(struct S19, f2), offsetof(struct S19, f3), offsetof(struct S19, f4));
    printf("S20: %zu %zu %zu %zu %zu\n", sizeof(struct S20), offsetof(struct S20, f0), offsetof(struct S20, f1), offsetof(struct S20, g0), offsetof(struct S20, g1));
    printf("S21: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S21), offsetof(struct S21, f0), offsetof(struct S21, f1), offsetof(struct S21, f2), offsetof(struct S21, f3), offsetof(struct S21, f4), offsetof(struct S21, g0), offsetof(struct S21, g1));
    printf("S22: %zu %zu %zu\n", sizeof(struct S22), offsetof(struct S22, f0), offsetof(struct S22, f1));
    printf("S23: %zu %zu %zu\n", sizeof(struct S23), offsetof(struct S23, f0), offsetof(struct S23, f1));
    printf("S24: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S24), offsetof(struct S24, f0), offsetof(struct S24, f1), offsetof(struct S24, f2), offsetof(struct S24, f3), offsetof(struct S24, f4), offsetof(struct S24, f5), offsetof(struct S24, f6));
    printf("S25: %zu %zu %zu %zu\n", sizeof(struct S25), offsetof(struct S25, f0), offsetof(struct S25, g0), offsetof(struct S25, g1));
    printf("S26: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S26), offsetof(struct S26, f0), offsetof(struct S26, f1), offsetof(struct S26, f2), offsetof(struct S26, f3), offsetof(struct S26, f4), offsetof(struct S26, f5), offsetof(struct S26, f6), offsetof(struct S26, f7));
    printf("S27: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S27), offsetof(struct S27, f0), offsetof(struct S27, f1), offsetof(struct S27, f2), offsetof(struct S27, f3), offsetof(struct S27, f4), offsetof(struct S27, f5), offsetof(struct S27, f6), offsetof(struct S27, g0), offsetof(struct S27, g1));
    printf("S28: %zu %zu %zu\n", sizeof(struct S28), offsetof(struct S28, f0), offsetof(struct S28, f1));
    printf("S29: %zu %zu %zu %zu %zu\n", sizeof(struct S29), offsetof(struct S29, f0), offsetof(struct S29, f1), offsetof(struct S29, g0), offsetof(struct S29, g1));
    printf("S30: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S30), offsetof(struct S30, f0), offsetof(struct S30, f1), offsetof(struct S30, f2), offsetof(struct S30, f3), offsetof(struct S30, f4), offsetof(struct S30, f5));
    printf("S31: %zu %zu %zu\n", sizeof(struct S31), offsetof(struct S31, f0), offsetof(struct S31, f1));
    printf("S32: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S32), offsetof(struct S32, f0), offsetof(struct S32, f1), offsetof(struct S32, f2), offsetof(struct S32, f3), offsetof(struct S32, g0), offsetof(struct S32, g1));
    printf("S33: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S33), offsetof(struct S33, f0), offsetof(struct S33, f1), offsetof(struct S33, f2), offsetof(struct S33, f3), offsetof(struct S33, f4), offsetof(struct S33, f5));
    printf("S34: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S34), offsetof(struct S34, f0), offsetof(struct S34, f1), offsetof(struct S34, f2), offsetof(struct S34, f3), offsetof(struct S34, f4), offsetof(struct S34, f5), offsetof(struct S34, f6));
    printf("S35: %zu %zu %zu %zu\n", sizeof(struct S35), offsetof(struct S35, f0), offsetof(struct S35, g0), offsetof(struct S35, g1));
    printf("S36: %zu %zu %zu %zu\n", sizeof(struct S36), offsetof(struct S36, f0), offsetof(struct S36, f1), offsetof(struct S36, f2));
    printf("S37: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S37), offsetof(struct S37, f0), offsetof(struct S37, f1), offsetof(struct S37, f2), offsetof(struct S37, f3), offsetof(struct S37, f4), offsetof(struct S37, f5), offsetof(struct S37, f6), offsetof(struct S37, f7));
    printf("S38: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S38), offsetof(struct S38, f0), offsetof(struct S38, f1), offsetof(struct S38, f2), offsetof(struct S38, f3), offsetof(struct S38, f4), offsetof(struct S38, g0), offsetof(struct S38, g1));
    printf("S39: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S39), offsetof(struct S39, f0), offsetof(struct S39, f1), offsetof(struct S39, f2), offsetof(struct S39, f3), offsetof(struct S39, f4), offsetof(struct S39, f5), offsetof(struct S39, f6));
    printf("S40: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S40), offsetof(struct S40, f0), offsetof(struct S40, f1), offsetof(struct S40, f2), offsetof(struct S40, f3), offsetof(struct S40, g0), offsetof(struct S40, g1));
    printf("S41: %zu %zu %zu\n", sizeof(struct S41), offsetof(struct S41, f0), offsetof(struct S41, f1));
    printf("S42: %zu %zu %zu %zu\n", sizeof(struct S42), offsetof(struct S42, f0), offsetof(struct S42, f1), offsetof(struct S42, f2));
    printf("S43: %zu %zu %zu %zu\n", sizeof(struct S43), offsetof(struct S43, f0), offsetof(struct S43, g0), offsetof(struct S43, g1));
    printf("S44: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S44), offsetof(struct S44, f0), offsetof(struct S44, f1), offsetof(struct S44, f2), offsetof(struct S44, f3), offsetof(struct S44, f4), offsetof(struct S44, f5), offsetof(struct S44, f6), offsetof(struct S44, f7), offsetof(struct S44, g0), offsetof(struct S44, g1));
    printf("S45: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S45), offsetof(struct S45, f0), offsetof(struct S45, f1), offsetof(struct S45, f2), offsetof(struct S45, f3), offsetof(struct S45, f4), offsetof(struct S45, f5), offsetof(struct S45, f6));
    printf("S46: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S46), offsetof(struct S46, f0), offsetof(struct S46, f1), offsetof(struct S46, f2), offsetof(struct S46, f3), offsetof(struct S46, f4), offsetof(struct S46, f5), offsetof(struct S46, f6), offsetof(struct S46, g0), offsetof(struct S46, g1));
    printf("S47: %zu %zu %zu %zu %zu\n", sizeof(struct S47), offsetof(struct S47, f0), offsetof(struct S47, f1), offsetof(struct S47, g0), offsetof(struct S47, g1));
    printf("S48: %zu %zu %zu\n", sizeof(struct S48), offsetof(struct S48, f0), offsetof(struct S48, f1));
    printf("S49: %zu %zu %zu %zu\n", sizeof(struct S49), offsetof(struct S49, f0), offsetof(struct S49, f1), offsetof(struct S49, f2));
    printf("S50: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S50), offsetof(struct S50, f0), offsetof(struct S50, f1), offsetof(struct S50, f2), offsetof(struct S50, f3), offsetof(struct S50, f4), offsetof(struct S50, f5), offsetof(struct S50, f6), offsetof(struct S50, g0), offsetof(struct S50, g1));
    printf("S51: %zu %zu %zu\n", sizeof(struct S51), offsetof(struct S51, f0), offsetof(struct S51, f1));
    printf("S52: %zu %zu\n", sizeof(struct S52), offsetof(struct S52, f0));
    printf("S53: %zu %zu %zu %zu\n", sizeof(struct S53), offsetof(struct S53, f0), offsetof(struct S53, f1), offsetof(struct S53, f2));
    printf("S54: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S54), offsetof(struct S54, f0), offsetof(struct S54, f1), offsetof(struct S54, f2), offsetof(struct S54, f3), offsetof(struct S54, f4));
    printf("S55: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S55), offsetof(struct S55, f0), offsetof(struct S55, f1), offsetof(struct S55, f2), offsetof(struct S55, f3), offsetof(struct S55, f4), offsetof(struct S55, f5), offsetof(struct S55, f6), offsetof(struct S55, f7));
    printf("S56: %zu %zu %zu %zu %zu\n", sizeof(struct S56), offsetof(struct S56, f0), offsetof(struct S56, f1), offsetof(struct S56, f2), offsetof(struct S56, f3));
    printf("S57: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S57), offsetof(struct S57, f0), offsetof(struct S57, f1), offsetof(struct S57, f2), offsetof(struct S57, f3), offsetof(struct S57, f4));
    printf("S58: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S58), offsetof(struct S58, f0), offsetof(struct S58, f1), offsetof(struct S58, f2), offsetof(struct S58, f3), offsetof(struct S58, f4), offsetof(struct S58, f5), offsetof(struct S58, f6), offsetof(struct S58, g0), offsetof(struct S58, g1));
    printf("S59: %zu %zu %zu %zu\n", sizeof(struct S59), offsetof(struct S59, f0), offsetof(struct S59, g0), offsetof(struct S59, g1));
    printf("S60: %zu %zu\n", sizeof(struct S60), offsetof(struct S60, f0));
    printf("S61: %zu %zu\n", sizeof(struct S61), offsetof(struct S61, f0));
    printf("S62: %zu %zu %zu\n", sizeof(struct S62), offsetof(struct S62, f0), offsetof(struct S62, f1));
    printf("S63: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S63), offsetof(struct S63, f0), offsetof(struct S63, f1), offsetof(struct S63, f2), offsetof(struct S63, f3), offsetof(struct S63, f4), offsetof(struct S63, f5), offsetof(struct S63, g0), offsetof(struct S63, g1));
    printf("S64: %zu %zu %zu %zu %zu\n", sizeof(struct S64), offsetof(struct S64, f0), offsetof(struct S64, f1), offsetof(struct S64, f2), offsetof(struct S64, f3));
    printf("S65: %zu %zu %zu %zu\n", sizeof(struct S65), offsetof(struct S65, f0), offsetof(struct S65, f1), offsetof(struct S65, f2));
    printf("S66: %zu %zu %zu %zu\n", sizeof(struct S66), offsetof(struct S66, f0), offsetof(struct S66, f1), offsetof(struct S66, f2));
    printf("S67: %zu %zu\n", sizeof(struct S67), offsetof(struct S67, f0));
    printf("S68: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S68), offsetof(struct S68, f0), offsetof(struct S68, f1), offsetof(struct S68, f2), offsetof(struct S68, f3), offsetof(struct S68, f4));
    printf("S69: %zu %zu %zu %zu\n", sizeof(struct S69), offsetof(struct S69, f0), offsetof(struct S69, f1), offsetof(struct S69, f2));
    printf("S70: %zu %zu %zu %zu %zu\n", sizeof(struct S70), offsetof(struct S70, f0), offsetof(struct S70, f1), offsetof(struct S70, g0), offsetof(struct S70, g1));
    printf("S71: %zu %zu %zu %zu %zu\n", sizeof(struct S71), offsetof(struct S71, f0), offsetof(struct S71, f1), offsetof(struct S71, f2), offsetof(struct S71, f3));
    printf("S72: %zu %zu\n", sizeof(struct S72), offsetof(struct S72, f0));
    printf("S73: %zu %zu %zu %zu\n", sizeof(struct S73), offsetof(struct S73, f0), offsetof(struct S73, f1), offsetof(struct S73, f2));
    printf("S74: %zu %zu\n", sizeof(struct S74), offsetof(struct S74, f0));
    printf("S75: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S75), offsetof(struct S75, f0), offsetof(struct S75, f1), offsetof(struct S75, f2), offsetof(struct S75, f3), offsetof(struct S75, f4), offsetof(struct S75, f5));
    printf("S76: %zu %zu %zu %zu %zu\n", sizeof(struct S76), offsetof(struct S76, f0), offsetof(struct S76, f1), offsetof(struct S76, f2), offsetof(struct S76, f3));
    printf("S77: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S77), offsetof(struct S77, f0), offsetof(struct S77, f1), offsetof(struct S77, f2), offsetof(struct S77, f3), offsetof(struct S77, f4), offsetof(struct S77, f5), offsetof(struct S77, f6), offsetof(struct S77, f7));
    printf("S78: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S78), offsetof(struct S78, f0), offsetof(struct S78, f1), offsetof(struct S78, f2), offsetof(struct S78, f3), offsetof(struct S78, f4), offsetof(struct S78, f5), offsetof(struct S78, f6));
    printf("S79: %zu %zu %zu %zu %zu\n", sizeof(struct S79), offsetof(struct S79, f0), offsetof(struct S79, f1), offsetof(struct S79, g0), offsetof(struct S79, g1));
    printf("S80: %zu %zu %zu %zu %zu\n", sizeof(struct S80), offsetof(struct S80, f0), offsetof(struct S80, f1), offsetof(struct S80, g0), offsetof(struct S80, g1));
    printf("S81: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S81), offsetof(struct S81, f0), offsetof(struct S81, f1), offsetof(struct S81, f2), offsetof(struct S81, f3), offsetof(struct S81, f4), offsetof(struct S81, f5), offsetof(struct S81, f6), offsetof(struct S81, f7));
    printf("S82: %zu %zu %zu %zu\n", sizeof(struct S82), offsetof(struct S82, f0), offsetof(struct S82, f1), offsetof(struct S82, f2));
    printf("S83: %zu %zu %zu %zu\n", sizeof(struct S83), offsetof(struct S83, f0), offsetof(struct S83, f1), offsetof(struct S83, f2));
    printf("S84: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S84), offsetof(struct S84, f0), offsetof(struct S84, f1), offsetof(struct S84, f2), offsetof(struct S84, f3), offsetof(struct S84, f4), offsetof(struct S84, f5), offsetof(struct S84, f6), offsetof(struct S84, f7), offsetof(struct S84, g0), offsetof(struct S84, g1));
    printf("S85: %zu %zu %zu\n", sizeof(struct S85), offsetof(struct S85, f0), offsetof(struct S85, f1));
    printf("S86: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S86), offsetof(struct S86, f0), offsetof(struct S86, f1), offsetof(struct S86, f2), offsetof(struct S86, f3), offsetof(struct S86, f4), offsetof(struct S86, f5), offsetof(struct S86, g0), offsetof(struct S86, g1));
    printf("S87: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S87), offsetof(struct S87, f0), offsetof(struct S87, f1), offsetof(struct S87, f2), offsetof(struct S87, f3), offsetof(struct S87, f4));
    printf("S88: %zu %zu %zu\n", sizeof(struct S88), offsetof(struct S88, f0), offsetof(struct S88, f1));
    printf("S89: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S89), offsetof(struct S89, f0), offsetof(struct S89, f1), offsetof(struct S89, f2), offsetof(struct S89, f3), offsetof(struct S89, f4), offsetof(struct S89, f5), offsetof(struct S89, f6));
    printf("S90: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S90), offsetof(struct S90, f0), offsetof(struct S90, f1), offsetof(struct S90, f2), offsetof(struct S90, f3), offsetof(struct S90, f4), offsetof(struct S90, f5));
    printf("S91: %zu %zu\n", sizeof(struct S91), offsetof(struct S91, f0));
    printf("S92: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S92), offsetof(struct S92, f0), offsetof(struct S92, f1), offsetof(struct S92, f2), offsetof(struct S92, f3), offsetof(struct S92, g0), offsetof(struct S92, g1));
    printf("S93: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S93), offsetof(struct S93, f0), offsetof(struct S93, f1), offsetof(struct S93, f2), offsetof(struct S93, f3), offsetof(struct S93, f4), offsetof(struct S93, g0), offsetof(struct S93, g1));
    printf("S94: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S94), offsetof(struct S94, f0), offsetof(struct S94, f1), offsetof(struct S94, f2), offsetof(struct S94, f3), offsetof(struct S94, f4), offsetof(struct S94, f5), offsetof(struct S94, f6), offsetof(struct S94, f7));
    printf("S95: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S95), offsetof(struct S95, f0), offsetof(struct S95, f1), offsetof(struct S95, f2), offsetof(struct S95, g0), offsetof(struct S95, g1));
    printf("S96: %zu %zu %zu %zu %zu\n", sizeof(struct S96), offsetof(struct S96, f0), offsetof(struct S96, f1), offsetof(struct S96, f2), offsetof(struct S96, f3));
    printf("S97: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S97), offsetof(struct S97, f0), offsetof(struct S97, f1), offsetof(struct S97, f2), offsetof(struct S97, f3), offsetof(struct S97, f4), offsetof(struct S97, f5), offsetof(struct S97, f6));
    printf("S98: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S98), offsetof(struct S98, f0), offsetof(struct S98, f1), offsetof(struct S98, f2), offsetof(struct S98, f3), offsetof(struct S98, f4), offsetof(struct S98, f5));
    printf("S99: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S99), offsetof(struct S99, f0), offsetof(struct S99, f1), offsetof(struct S99, f2), offsetof(struct S99, f3), offsetof(struct S99, g0), offsetof(struct S99, g1));
    printf("S100: %zu %zu %zu %zu\n", sizeof(struct S100), offsetof(struct S100, f0), offsetof(struct S100, g0), offsetof(struct S100, g1));
    printf("S101: %zu %zu %zu %zu\n", sizeof(struct S101), offsetof(struct S101, f0), offsetof(struct S101, f1), offsetof(struct S101, f2));
    printf("S102: %zu %zu %zu %zu %zu\n", sizeof(struct S102), offsetof(struct S102, f0), offsetof(struct S102, f1), offsetof(struct S102, f2), offsetof(struct S102, f3));
    printf("S103: %zu %zu\n", sizeof(struct S103), offsetof(struct S103, f0));
    printf("S104: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S104), offsetof(struct S104, f0), offsetof(struct S104, f1), offsetof(struct S104, f2), offsetof(struct S104, f3), offsetof(struct S104, f4), offsetof(struct S104, f5), offsetof(struct S104, f6), offsetof(struct S104, f7));
    printf("S105: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S105), offsetof(struct S105, f0), offsetof(struct S105, f1), offsetof(struct S105, f2), offsetof(struct S105, f3), offsetof(struct S105, f4), offsetof(struct S105, f5), offsetof(struct S105, g0), offsetof(struct S105, g1));
    printf("S106: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S106), offsetof(struct S106, f0), offsetof(struct S106, f1), offsetof(struct S106, f2), offsetof(struct S106, f3), offsetof(struct S106, f4), offsetof(struct S106, f5), offsetof(struct S106, f6), offsetof(struct S106, f7), offsetof(struct S106, g0), offsetof(struct S106, g1));
    printf("S107: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S107), offsetof(struct S107, f0), offsetof(struct S107, f1), offsetof(struct S107, f2), offsetof(struct S107, f3), offsetof(struct S107, f4), offsetof(struct S107, f5), offsetof(struct S107, f6));
    printf("S108: %zu %zu\n", sizeof(struct S108), offsetof(struct S108, f0));
    printf("S109: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S109), offsetof(struct S109, f0), offsetof(struct S109, f1), offsetof(struct S109, f2), offsetof(struct S109, g0), offsetof(struct S109, g1));
    printf("S110: %zu %zu %zu %zu %zu\n", sizeof(struct S110), offsetof(struct S110, f0), offsetof(struct S110, f1), offsetof(struct S110, f2), offsetof(struct S110, f3));
    printf("S111: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S111), offsetof(struct S111, f0), offsetof(struct S111, f1), offsetof(struct S111, f2), offsetof(struct S111, f3), offsetof(struct S111, f4), offsetof(struct S111, f5), offsetof(struct S111, f6), offsetof(struct S111, f7));
    printf("S112: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S112), offsetof(struct S112, f0), offsetof(struct S112, f1), offsetof(struct S112, f2), offsetof(struct S112, f3), offsetof(struct S112, f4), offsetof(struct S112, f5), offsetof(struct S112, f6), offsetof(struct S112, f7));
    printf("S113: %zu %zu %zu %zu %zu\n", sizeof(struct S113), offsetof(struct S113, f0), offsetof(struct S113, f1), offsetof(struct S113, f2), offsetof(struct S113, f3));
    printf("S114: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S114), offsetof(struct S114, f0), offsetof(struct S114, f1), offsetof(struct S114, f2), offsetof(struct S114, f3), offsetof(struct S114, f4), offsetof(struct S114, f5), offsetof(struct S114, f6), offsetof(struct S114, f7));
    printf("S115: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S115), offsetof(struct S115, f0), offsetof(struct S115, f1), offsetof(struct S115, f2), offsetof(struct S115, f3), offsetof(struct S115, f4), offsetof(struct S115, f5), offsetof(struct S115, f6), offsetof(struct S115, f7));
    printf("S116: %zu %zu %zu %zu\n", sizeof(struct S116), offsetof(struct S116, f0), offsetof(struct S116, f1), offsetof(struct S116, f2));
    printf("S117: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S117), offsetof(struct S117, f0), offsetof(struct S117, f1), offsetof(struct S117, f2), offsetof(struct S117, f3), offsetof(struct S117, f4), offsetof(struct S117, f5));
    printf("S118: %zu %zu %zu\n", sizeof(struct S118), offsetof(struct S118, f0), offsetof(struct S118, f1));
    printf("S119: %zu %zu %zu\n", sizeof(struct S119), offsetof(struct S119, f0), offsetof(struct S119, f1));
    printf("S120: %zu %zu %zu\n", sizeof(struct S120), offsetof(struct S120, f0), offsetof(struct S120, f1));
    printf("S121: %zu %zu %zu %zu\n", sizeof(struct S121), offsetof(struct S121, f0), offsetof(struct S121, f1), offsetof(struct S121, f2));
    printf("S122: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S122), offsetof(struct S122, f0), offsetof(struct S122, f1), offsetof(struct S122, f2), offsetof(struct S122, f3), offsetof(struct S122, f4), offsetof(struct S122, f5), offsetof(struct S122, f6), offsetof(struct S122, f7), offsetof(struct S122, g0), offsetof(struct S122, g1));
    printf("S123: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S123), offsetof(struct S123, f0), offsetof(struct S123, f1), offsetof(struct S123, f2), offsetof(struct S123, g0), offsetof(struct S123, g1));
    printf("S124: %zu %zu\n", sizeof(struct S124), offsetof(struct S124, f0));
    printf("S125: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S125), offsetof(struct S125, f0), offsetof(struct S125, f1), offsetof(struct S125, f2), offsetof(struct S125, f3), offsetof(struct S125, f4), offsetof(struct S125, f5));
    printf("S126: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S126), offsetof(struct S126, f0), offsetof(struct S126, f1), offsetof(struct S126, f2), offsetof(struct S126, f3), offsetof(struct S126, f4), offsetof(struct S126, g0), offsetof(struct S126, g1));
    printf("S127: %zu %zu\n", sizeof(struct S127), offsetof(struct S127, f0));
    printf("S128: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S128), offsetof(struct S128, f0), offsetof(struct S128, f1), offsetof(struct S128, f2), offsetof(struct S128, f3), offsetof(struct S128, f4), offsetof(struct S128, f5));
    printf("S129: %zu %zu %zu\n", sizeof(struct S129), offsetof(struct S129, f0), offsetof(struct S129, f1));
    printf("S130: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S130), offsetof(struct S130, f0), offsetof(struct S130, f1), offsetof(struct S130, f2), offsetof(struct S130, f3), offsetof(struct S130, f4), offsetof(struct S130, g0), offsetof(struct S130, g1));
    printf("S131: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S131), offsetof(struct S131, f0), offsetof(struct S131, f1), offsetof(struct S131, f2), offsetof(struct S131, f3), offsetof(struct S131, f4), offsetof(struct S131, f5), offsetof(struct S131, g0), offsetof(struct S131, g1));
    printf("S132: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S132), offsetof(struct S132, f0), offsetof(struct S132, f1), offsetof(struct S132, f2), offsetof(struct S132, f3), offsetof(struct S132, f4), offsetof(struct S132, f5), offsetof(struct S132, f6), offsetof(struct S132, f7));
    printf("S133: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S133), offsetof(struct S133, f0), offsetof(struct S133, f1), offsetof(struct S133, f2), offsetof(struct S133, f3), offsetof(struct S133, f4), offsetof(struct S133, f5), offsetof(struct S133, f6), offsetof(struct S133, f7));
    printf("S134: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S134), offsetof(struct S134, f0), offsetof(struct S134, f1), offsetof(struct S134, f2), offsetof(struct S134, f3), offsetof(struct S134, f4), offsetof(struct S134, g0), offsetof(struct S134, g1));
    printf("S135: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S135), offsetof(struct S135, f0), offsetof(struct S135, f1), offsetof(struct S135, f2), offsetof(struct S135, f3), offsetof(struct S135, g0), offsetof(struct S135, g1));
    printf("S136: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S136), offsetof(struct S136, f0), offsetof(struct S136, f1), offsetof(struct S136, f2), offsetof(struct S136, f3), offsetof(struct S136, f4), offsetof(struct S136, f5), offsetof(struct S136, f6), offsetof(struct S136, f7), offsetof(struct S136, g0), offsetof(struct S136, g1));
    printf("S137: %zu %zu %zu %zu %zu\n", sizeof(struct S137), offsetof(struct S137, f0), offsetof(struct S137, f1), offsetof(struct S137, f2), offsetof(struct S137, f3));
    printf("S138: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S138), offsetof(struct S138, f0), offsetof(struct S138, f1), offsetof(struct S138, f2), offsetof(struct S138, f3), offsetof(struct S138, f4), offsetof(struct S138, f5), offsetof(struct S138, f6), offsetof(struct S138, f7), offsetof(struct S138, g0), offsetof(struct S138, g1));
    printf("S139: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S139), offsetof(struct S139, f0), offsetof(struct S139, f1), offsetof(struct S139, f2), offsetof(struct S139, f3), offsetof(struct S139, f4), offsetof(struct S139, f5), offsetof(struct S139, f6), offsetof(struct S139, g0), offsetof(struct S139, g1));
    printf("S140: %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S140), offsetof(struct S140, f0), offsetof(struct S140, f1), offsetof(struct S140, f2), offsetof(struct S140, f3), offsetof(struct S140, f4), offsetof(struct S140, f5));
    printf("S141: %zu %zu\n", sizeof(struct S141), offsetof(struct S141, f0));
    printf("S142: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S142), offsetof(struct S142, f0), offsetof(struct S142, f1), offsetof(struct S142, f2), offsetof(struct S142, f3), offsetof(struct S142, f4), offsetof(struct S142, f5), offsetof(struct S142, f6), offsetof(struct S142, f7));
    printf("S143: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S143), offsetof(struct S143, f0), offsetof(struct S143, f1), offsetof(struct S143, f2), offsetof(struct S143, f3), offsetof(struct S143, f4), offsetof(struct S143, f5), offsetof(struct S143, f6), offsetof(struct S143, f7));
    printf("S144: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S144), offsetof(struct S144, f0), offsetof(struct S144, f1), offsetof(struct S144, f2), offsetof(struct S144, f3), offsetof(struct S144, f4));
    printf("S145: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S145), offsetof(struct S145, f0), offsetof(struct S145, f1), offsetof(struct S145, f2), offsetof(struct S145, f3), offsetof(struct S145, f4), offsetof(struct S145, f5), offsetof(struct S145, f6), offsetof(struct S145, g0), offsetof(struct S145, g1));
    printf("S146: %zu %zu\n", sizeof(struct S146), offsetof(struct S146, f0));
    printf("S147: %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S147), offsetof(struct S147, f0), offsetof(struct S147, f1), offsetof(struct S147, f2), offsetof(struct S147, f3), offsetof(struct S147, f4), offsetof(struct S147, f5), offsetof(struct S147, f6), offsetof(struct S147, f7), offsetof(struct S147, g0), offsetof(struct S147, g1));
    printf("S148: %zu %zu %zu %zu %zu\n", sizeof(struct S148), offsetof(struct S148, f0), offsetof(struct S148, f1), offsetof(struct S148, f2), offsetof(struct S148, f3));
    printf("S149: %zu %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S149), offsetof(struct S149, f0), offsetof(struct S149, f1), offsetof(struct S149, f2), offsetof(struct S149, f3), offsetof(struct S149, f4), offsetof(struct S149, f5), offsetof(struct S149, g0), offsetof(struct S149, g1));
    printf("S150: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S150), offsetof(struct S150, f0), offsetof(struct S150, f1), offsetof(struct S150, f2), offsetof(struct S150, f3), offsetof(struct S150, f4));
    printf("S151: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S151), offsetof(struct S151, f0), offsetof(struct S151, f1), offsetof(struct S151, f2), offsetof(struct S151, f3), offsetof(struct S151, f4));
    printf("S152: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S152), offsetof(struct S152, f0), offsetof(struct S152, f1), offsetof(struct S152, f2), offsetof(struct S152, f3), offsetof(struct S152, f4), offsetof(struct S152, f5), offsetof(struct S152, f6));
    printf("S153: %zu %zu %zu %zu %zu\n", sizeof(struct S153), offsetof(struct S153, f0), offsetof(struct S153, f1), offsetof(struct S153, f2), offsetof(struct S153, f3));
    printf("S154: %zu %zu %zu %zu\n", sizeof(struct S154), offsetof(struct S154, f0), offsetof(struct S154, g0), offsetof(struct S154, g1));
    printf("S155: %zu %zu %zu %zu %zu\n", sizeof(struct S155), offsetof(struct S155, f0), offsetof(struct S155, f1), offsetof(struct S155, g0), offsetof(struct S155, g1));
    printf("S156: %zu %zu %zu %zu %zu %zu\n", sizeof(struct S156), offsetof(struct S156, f0), offsetof(struct S156, f1), offsetof(struct S156, f2), offsetof(struct S156, g0), offsetof(struct S156, g1));
    printf("S157: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S157), offsetof(struct S157, f0), offsetof(struct S157, f1), offsetof(struct S157, f2), offsetof(struct S157, f3), offsetof(struct S157, f4), offsetof(struct S157, f5), offsetof(struct S157, f6));
    printf("S158: %zu %zu %zu %zu %zu\n", sizeof(struct S158), offsetof(struct S158, f0), offsetof(struct S158, f1), offsetof(struct S158, g0), offsetof(struct S158, g1));
    printf("S159: %zu %zu %zu %zu %zu %zu %zu %zu\n", sizeof(struct S159), offsetof(struct S159, f0), offsetof(struct S159, f1), offsetof(struct S159, f2), offsetof(struct S159, f3), offsetof(struct S159, f4), offsetof(struct S159, f5), offsetof(struct S159, f6));
    return 0;
}
