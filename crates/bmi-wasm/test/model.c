/* Toy "leaky bucket" BMI model used to test the wasm adapter end-to-end.
 * Written like a normal native BMI model: reads its config file with fopen,
 * keeps state behind bmi->data, exposes double/float/int variables.
 *
 * Native:  gcc -shared -fPIC -o model.so model.c
 * Wasm:    emcc -O2 -sSIDE_MODULE=2 -o model.wasm model.c
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "bmi.h"

#ifdef __EMSCRIPTEN__
#include <emscripten.h>
#define EXPORT EMSCRIPTEN_KEEPALIVE
#else
#define EXPORT
#endif

typedef struct {
    double storage;      /* output, double */
    double precip;       /* input, double  */
    float outflow;       /* output, float  */
    int iterations;      /* output, int    */
    double leak_rate;
    double current_time;
    double time_step;
    double end_time;
} LeakyBucket;

#define SELF ((LeakyBucket *)self->data)

static const char *INPUT_NAMES[] = {"atmosphere_water__precipitation_leq-volume_flux"};
static const char *OUTPUT_NAMES[] = {"bucket__water_storage", "bucket__outflow", "bucket__iterations"};

static int model_initialize(Bmi *self, const char *config_file) {
    FILE *fp = fopen(config_file, "r");
    if (!fp) return BMI_FAILURE;

    LeakyBucket *m = SELF;
    m->storage = 0.0;
    m->leak_rate = 0.1;
    char line[256];
    while (fgets(line, sizeof line, fp)) {
        double v;
        if (sscanf(line, "initial_storage=%lf", &v) == 1) m->storage = v;
        if (sscanf(line, "leak_rate=%lf", &v) == 1) m->leak_rate = v;
    }
    fclose(fp);

    m->precip = 0.0;
    m->outflow = 0.0f;
    m->iterations = 0;
    m->current_time = 0.0;
    m->time_step = 3600.0;
    m->end_time = 86400.0;
    return BMI_SUCCESS;
}

static int model_update(Bmi *self) {
    LeakyBucket *m = SELF;
    m->storage += m->precip * m->time_step;
    double leaked = m->storage * m->leak_rate;
    m->storage -= leaked;
    m->outflow = (float)(leaked / m->time_step);
    m->iterations += 1;
    m->current_time += m->time_step;
    return BMI_SUCCESS;
}

static int model_update_until(Bmi *self, double then) {
    while (SELF->current_time < then) {
        if (model_update(self) != BMI_SUCCESS) return BMI_FAILURE;
    }
    return BMI_SUCCESS;
}

static int model_finalize(Bmi *self) {
    (void)self;
    return BMI_SUCCESS;
}

static int get_component_name(Bmi *self, char *name) {
    (void)self;
    strcpy(name, "Leaky Bucket (wasm test model)");
    return BMI_SUCCESS;
}

static int get_input_item_count(Bmi *self, int *count) {
    (void)self;
    *count = 1;
    return BMI_SUCCESS;
}

static int get_output_item_count(Bmi *self, int *count) {
    (void)self;
    *count = 3;
    return BMI_SUCCESS;
}

static int get_input_var_names(Bmi *self, char **names) {
    (void)self;
    for (int i = 0; i < 1; i++) strcpy(names[i], INPUT_NAMES[i]);
    return BMI_SUCCESS;
}

static int get_output_var_names(Bmi *self, char **names) {
    (void)self;
    for (int i = 0; i < 3; i++) strcpy(names[i], OUTPUT_NAMES[i]);
    return BMI_SUCCESS;
}

static int get_var_grid(Bmi *self, const char *name, int *grid) {
    (void)self; (void)name;
    *grid = 0;
    return BMI_SUCCESS;
}

static int get_var_type(Bmi *self, const char *name, char *type) {
    (void)self;
    if (strcmp(name, "bucket__outflow") == 0) strcpy(type, "float");
    else if (strcmp(name, "bucket__iterations") == 0) strcpy(type, "int");
    else strcpy(type, "double");
    return BMI_SUCCESS;
}

static int get_var_units(Bmi *self, const char *name, char *units) {
    (void)self;
    if (strcmp(name, "bucket__outflow") == 0) strcpy(units, "m s-1");
    else if (strcmp(name, "bucket__iterations") == 0) strcpy(units, "1");
    else if (strcmp(name, "bucket__water_storage") == 0) strcpy(units, "m");
    else strcpy(units, "m s-1");
    return BMI_SUCCESS;
}

static int get_var_itemsize(Bmi *self, const char *name, int *size) {
    (void)self;
    if (strcmp(name, "bucket__outflow") == 0) *size = sizeof(float);
    else if (strcmp(name, "bucket__iterations") == 0) *size = sizeof(int);
    else *size = sizeof(double);
    return BMI_SUCCESS;
}

static int get_var_nbytes(Bmi *self, const char *name, int *nbytes) {
    int itemsize;
    get_var_itemsize(self, name, &itemsize);
    *nbytes = itemsize; /* all vars are scalars */
    return BMI_SUCCESS;
}

static int get_var_location(Bmi *self, const char *name, char *location) {
    (void)self; (void)name;
    strcpy(location, "node");
    return BMI_SUCCESS;
}

static int get_current_time(Bmi *self, double *time) {
    *time = SELF->current_time;
    return BMI_SUCCESS;
}

static int get_start_time(Bmi *self, double *time) {
    (void)self;
    *time = 0.0;
    return BMI_SUCCESS;
}

static int get_end_time(Bmi *self, double *time) {
    *time = SELF->end_time;
    return BMI_SUCCESS;
}

static int get_time_units(Bmi *self, char *units) {
    (void)self;
    strcpy(units, "s");
    return BMI_SUCCESS;
}

static int get_time_step(Bmi *self, double *time_step) {
    *time_step = SELF->time_step;
    return BMI_SUCCESS;
}

static int get_value(Bmi *self, const char *name, void *dest) {
    LeakyBucket *m = SELF;
    if (strcmp(name, "bucket__water_storage") == 0) {
        *(double *)dest = m->storage;
    } else if (strcmp(name, "bucket__outflow") == 0) {
        *(float *)dest = m->outflow;
    } else if (strcmp(name, "bucket__iterations") == 0) {
        *(int *)dest = m->iterations;
    } else if (strcmp(name, INPUT_NAMES[0]) == 0) {
        *(double *)dest = m->precip;
    } else {
        return BMI_FAILURE;
    }
    return BMI_SUCCESS;
}

static int set_value(Bmi *self, const char *name, void *src) {
    if (strcmp(name, INPUT_NAMES[0]) != 0) return BMI_FAILURE;
    SELF->precip = *(double *)src;
    return BMI_SUCCESS;
}

static int get_grid_rank(Bmi *self, int grid, int *rank) {
    (void)self; (void)grid;
    *rank = 0;
    return BMI_SUCCESS;
}

static int get_grid_size(Bmi *self, int grid, int *size) {
    (void)self; (void)grid;
    *size = 1;
    return BMI_SUCCESS;
}

static int get_grid_type(Bmi *self, int grid, char *type) {
    (void)self; (void)grid;
    strcpy(type, "scalar");
    return BMI_SUCCESS;
}

EXPORT Bmi *register_bmi_leaky_bucket(Bmi *model) {
    if (!model) return NULL;
    model->data = calloc(1, sizeof(LeakyBucket));

    model->initialize = model_initialize;
    model->update = model_update;
    model->update_until = model_update_until;
    model->finalize = model_finalize;

    model->get_component_name = get_component_name;
    model->get_input_item_count = get_input_item_count;
    model->get_output_item_count = get_output_item_count;
    model->get_input_var_names = get_input_var_names;
    model->get_output_var_names = get_output_var_names;

    model->get_var_grid = get_var_grid;
    model->get_var_type = get_var_type;
    model->get_var_units = get_var_units;
    model->get_var_itemsize = get_var_itemsize;
    model->get_var_nbytes = get_var_nbytes;
    model->get_var_location = get_var_location;

    model->get_current_time = get_current_time;
    model->get_start_time = get_start_time;
    model->get_end_time = get_end_time;
    model->get_time_units = get_time_units;
    model->get_time_step = get_time_step;

    model->get_value = get_value;
    model->set_value = set_value;

    model->get_grid_rank = get_grid_rank;
    model->get_grid_size = get_grid_size;
    model->get_grid_type = get_grid_type;

    return model;
}
