/* includes: next to the file, nested from the included file's own
   directory, guarded, #pragma once, and through -I */
#include "inc/a.h"
#include "inc/a.h"
#include "inc/once.h"
#include "inc/once.h"
#include <found_by_I.h>
#include "found_by_I.h"
int values[] = { A_VALUE, ONCE_VALUE, I_VALUE, NESTED };
