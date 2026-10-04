/*
 * Link-time stand-in for libSceMouse (the payload SDK has no stub for it):
 * the eboot imports these names from libSceMouse.prx; this body is never run.
 */
int sceMouseInit(void) { return -1; }
int sceMouseOpen(int user, int type, int index, const void *param) { (void)user; (void)type; (void)index; (void)param; return -1; }
int sceMouseRead(int handle, void *data, int count) { (void)handle; (void)data; (void)count; return -1; }
int sceMouseClose(int handle) { (void)handle; return -1; }
