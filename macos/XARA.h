#include <stddef.h>
#include <stdint.h>
int32_t xara_embedded_start(const char *config, const char *directory);
void xara_embedded_stop(void);
void xara_embedded_wait(void);
int32_t xara_embedded_status(char *output, size_t capacity);
size_t xara_embedded_devices(char *output, size_t capacity);
int32_t xara_embedded_can_configure(void);
int32_t xara_embedded_valid_call(const char *call);

size_t xara_diagnostics_init(const char *directory, char *output, size_t capacity);
void xara_diagnostics_event(const char *text);
size_t xara_diagnostics_export(char *output, size_t capacity);
