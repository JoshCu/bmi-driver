/*
 * A minimal BMI model for testing the driver end to end, without needing any real hydrological
 * model libraries to be present.
 *
 * It is a one-bucket store: precipitation goes in, a fixed fraction drains out each timestep.
 * The variables deliberately use three different C types (double, float, int) so that the
 * adapters' type dispatch is exercised, and the units deliberately differ from the forcings'
 * so that unit conversion is exercised too.
 *
 * Build:  cc -shared -fPIC -o libbucketbmi.so bucket_bmi.c
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define BMI_SUCCESS 0
#define BMI_FAILURE 1
#define BMI_MAX_NAME 2048

struct Bmi;

typedef struct Bmi {
    void *data;

    int (*initialize)(struct Bmi *self, const char *config_file);
    int (*update)(struct Bmi *self);
    int (*update_until)(struct Bmi *self, double then);
    int (*finalize)(struct Bmi *self);

    int (*get_component_name)(struct Bmi *self, char *name);
    int (*get_input_item_count)(struct Bmi *self, int *count);
    int (*get_output_item_count)(struct Bmi *self, int *count);
    int (*get_input_var_names)(struct Bmi *self, char **names);
    int (*get_output_var_names)(struct Bmi *self, char **names);

    int (*get_var_grid)(struct Bmi *self, const char *name, int *grid);
    int (*get_var_type)(struct Bmi *self, const char *name, char *type);
    int (*get_var_units)(struct Bmi *self, const char *name, char *units);
    int (*get_var_itemsize)(struct Bmi *self, const char *name, int *size);
    int (*get_var_nbytes)(struct Bmi *self, const char *name, int *nbytes);
    int (*get_var_location)(struct Bmi *self, const char *name, char *location);

    int (*get_current_time)(struct Bmi *self, double *time);
    int (*get_start_time)(struct Bmi *self, double *time);
    int (*get_end_time)(struct Bmi *self, double *time);
    int (*get_time_units)(struct Bmi *self, char *units);
    int (*get_time_step)(struct Bmi *self, double *step);

    int (*get_value)(struct Bmi *self, const char *name, void *dest);
    int (*get_value_ptr)(struct Bmi *self, const char *name, void **dest);
    int (*get_value_at_indices)(struct Bmi *self, const char *name, void *dest, int *inds, int len);

    int (*set_value)(struct Bmi *self, const char *name, void *src);
    int (*set_value_at_indices)(struct Bmi *self, const char *name, int *inds, int len, void *src);

    int (*get_grid_rank)(struct Bmi *self, int grid, int *rank);
    int (*get_grid_size)(struct Bmi *self, int grid, int *size);
    int (*get_grid_type)(struct Bmi *self, int grid, char *type);
    int (*get_grid_shape)(struct Bmi *self, int grid, int *shape);
    int (*get_grid_spacing)(struct Bmi *self, int grid, double *spacing);
    int (*get_grid_origin)(struct Bmi *self, int grid, double *origin);
    int (*get_grid_x)(struct Bmi *self, int grid, double *x);
    int (*get_grid_y)(struct Bmi *self, int grid, double *y);
    int (*get_grid_z)(struct Bmi *self, int grid, double *z);
    int (*get_grid_node_count)(struct Bmi *self, int grid, int *count);
    int (*get_grid_edge_count)(struct Bmi *self, int grid, int *count);
    int (*get_grid_face_count)(struct Bmi *self, int grid, int *count);
    int (*get_grid_edge_nodes)(struct Bmi *self, int grid, int *nodes);
    int (*get_grid_face_edges)(struct Bmi *self, int grid, int *edges);
    int (*get_grid_face_nodes)(struct Bmi *self, int grid, int *nodes);
    int (*get_grid_nodes_per_face)(struct Bmi *self, int grid, int *count);
} Bmi;

/* --- model state --- */

typedef struct {
    double precip_rate;   /* input,  m h-1  */
    float temperature;    /* input,  degC   */
    double storage;       /* output, m      */
    double q_out;         /* output, m h-1  */
    int step_count;       /* output, count  */
    double drain_fraction;
    double time;
    double dt;
} Bucket;

static const char *INPUT_NAMES[] = {"precip_rate", "temperature"};
static const char *OUTPUT_NAMES[] = {"Q_OUT", "STORAGE", "STEP_COUNT"};
#define N_INPUTS 2
#define N_OUTPUTS 3

static Bucket *state(Bmi *self) { return (Bucket *)self->data; }

static int b_initialize(Bmi *self, const char *config_file) {
    Bucket *s = state(self);
    s->precip_rate = 0.0;
    s->temperature = 0.0f;
    s->storage = 0.0;
    s->q_out = 0.0;
    s->step_count = 0;
    s->drain_fraction = 0.1;
    s->time = 0.0;
    s->dt = 1.0; /* one hour, see get_time_units */

    /* config is a trivial "key=value" file; only drain_fraction is understood */
    FILE *f = fopen(config_file, "r");
    if (f) {
        char key[256];
        double value;
        while (fscanf(f, "%255[^=]=%lf\n", key, &value) == 2) {
            if (strncmp(key, "drain_fraction", 14) == 0) s->drain_fraction = value;
            if (strncmp(key, "initial_storage", 15) == 0) s->storage = value;
        }
        fclose(f);
    }
    return BMI_SUCCESS;
}

static int b_update(Bmi *self) {
    Bucket *s = state(self);
    s->storage += s->precip_rate * s->dt;
    s->q_out = s->storage * s->drain_fraction;
    s->storage -= s->q_out;
    s->time += s->dt;
    s->step_count += 1;
    return BMI_SUCCESS;
}

static int b_update_until(Bmi *self, double then) {
    Bucket *s = state(self);
    while (s->time < then) {
        if (b_update(self) != BMI_SUCCESS) return BMI_FAILURE;
    }
    return BMI_SUCCESS;
}

static int b_finalize(Bmi *self) {
    if (self->data) {
        free(self->data);
        self->data = NULL;
    }
    return BMI_SUCCESS;
}

static int b_component_name(Bmi *self, char *name) {
    (void)self;
    strncpy(name, "Bucket", BMI_MAX_NAME - 1);
    return BMI_SUCCESS;
}

static int b_input_count(Bmi *self, int *count) { (void)self; *count = N_INPUTS; return BMI_SUCCESS; }
static int b_output_count(Bmi *self, int *count) { (void)self; *count = N_OUTPUTS; return BMI_SUCCESS; }

static int b_input_names(Bmi *self, char **names) {
    (void)self;
    for (int i = 0; i < N_INPUTS; i++) strncpy(names[i], INPUT_NAMES[i], BMI_MAX_NAME - 1);
    return BMI_SUCCESS;
}

static int b_output_names(Bmi *self, char **names) {
    (void)self;
    for (int i = 0; i < N_OUTPUTS; i++) strncpy(names[i], OUTPUT_NAMES[i], BMI_MAX_NAME - 1);
    return BMI_SUCCESS;
}

static int b_var_grid(Bmi *self, const char *name, int *grid) {
    (void)self; (void)name;
    *grid = 0;
    return BMI_SUCCESS;
}

static int b_var_type(Bmi *self, const char *name, char *type) {
    (void)self;
    if (strcmp(name, "temperature") == 0) strcpy(type, "float");
    else if (strcmp(name, "STEP_COUNT") == 0) strcpy(type, "int");
    else strcpy(type, "double");
    return BMI_SUCCESS;
}

static int b_var_units(Bmi *self, const char *name, char *units) {
    (void)self;
    if (strcmp(name, "temperature") == 0) strcpy(units, "degC");
    else if (strcmp(name, "STEP_COUNT") == 0) strcpy(units, "1");
    else if (strcmp(name, "STORAGE") == 0) strcpy(units, "m");
    else strcpy(units, "m h-1");
    return BMI_SUCCESS;
}

static int b_var_itemsize(Bmi *self, const char *name, int *size) {
    (void)self;
    if (strcmp(name, "temperature") == 0) *size = (int)sizeof(float);
    else if (strcmp(name, "STEP_COUNT") == 0) *size = (int)sizeof(int);
    else *size = (int)sizeof(double);
    return BMI_SUCCESS;
}

static int b_var_nbytes(Bmi *self, const char *name, int *nbytes) {
    return b_var_itemsize(self, name, nbytes);
}

static int b_var_location(Bmi *self, const char *name, char *location) {
    (void)self; (void)name;
    strcpy(location, "node");
    return BMI_SUCCESS;
}

static int b_current_time(Bmi *self, double *t) { *t = state(self)->time; return BMI_SUCCESS; }
static int b_start_time(Bmi *self, double *t) { (void)self; *t = 0.0; return BMI_SUCCESS; }
static int b_end_time(Bmi *self, double *t) { (void)self; *t = 1e9; return BMI_SUCCESS; }
static int b_time_step(Bmi *self, double *t) { *t = state(self)->dt; return BMI_SUCCESS; }
static int b_time_units(Bmi *self, char *units) { (void)self; strcpy(units, "h"); return BMI_SUCCESS; }

static int b_get_value(Bmi *self, const char *name, void *dest) {
    Bucket *s = state(self);
    if (strcmp(name, "Q_OUT") == 0) *(double *)dest = s->q_out;
    else if (strcmp(name, "STORAGE") == 0) *(double *)dest = s->storage;
    else if (strcmp(name, "STEP_COUNT") == 0) *(int *)dest = s->step_count;
    else if (strcmp(name, "precip_rate") == 0) *(double *)dest = s->precip_rate;
    else if (strcmp(name, "temperature") == 0) *(float *)dest = s->temperature;
    else return BMI_FAILURE;
    return BMI_SUCCESS;
}

static int b_get_value_ptr(Bmi *self, const char *name, void **dest) {
    Bucket *s = state(self);
    if (strcmp(name, "Q_OUT") == 0) *dest = &s->q_out;
    else if (strcmp(name, "STORAGE") == 0) *dest = &s->storage;
    else if (strcmp(name, "STEP_COUNT") == 0) *dest = &s->step_count;
    else if (strcmp(name, "precip_rate") == 0) *dest = &s->precip_rate;
    else if (strcmp(name, "temperature") == 0) *dest = &s->temperature;
    else return BMI_FAILURE;
    return BMI_SUCCESS;
}

static int b_set_value(Bmi *self, const char *name, void *src) {
    Bucket *s = state(self);
    if (strcmp(name, "precip_rate") == 0) s->precip_rate = *(double *)src;
    else if (strcmp(name, "temperature") == 0) s->temperature = *(float *)src;
    else if (strcmp(name, "STORAGE") == 0) s->storage = *(double *)src;
    else if (strcmp(name, "drain_fraction") == 0) s->drain_fraction = *(double *)src;
    else if (strcmp(name, "STEP_COUNT") == 0) s->step_count = *(int *)src;
    else return BMI_FAILURE;
    return BMI_SUCCESS;
}

static int b_grid_rank(Bmi *self, int grid, int *rank) { (void)self; (void)grid; *rank = 0; return BMI_SUCCESS; }
static int b_grid_size(Bmi *self, int grid, int *size) { (void)self; (void)grid; *size = 1; return BMI_SUCCESS; }
static int b_grid_type(Bmi *self, int grid, char *type) { (void)self; (void)grid; strcpy(type, "scalar"); return BMI_SUCCESS; }

Bmi *register_bmi_bucket(Bmi *model) {
    if (!model) return NULL;
    memset(model, 0, sizeof(Bmi));
    model->data = calloc(1, sizeof(Bucket));

    model->initialize = b_initialize;
    model->update = b_update;
    model->update_until = b_update_until;
    model->finalize = b_finalize;

    model->get_component_name = b_component_name;
    model->get_input_item_count = b_input_count;
    model->get_output_item_count = b_output_count;
    model->get_input_var_names = b_input_names;
    model->get_output_var_names = b_output_names;

    model->get_var_grid = b_var_grid;
    model->get_var_type = b_var_type;
    model->get_var_units = b_var_units;
    model->get_var_itemsize = b_var_itemsize;
    model->get_var_nbytes = b_var_nbytes;
    model->get_var_location = b_var_location;

    model->get_current_time = b_current_time;
    model->get_start_time = b_start_time;
    model->get_end_time = b_end_time;
    model->get_time_units = b_time_units;
    model->get_time_step = b_time_step;

    model->get_value = b_get_value;
    model->get_value_ptr = b_get_value_ptr;
    model->set_value = b_set_value;

    model->get_grid_rank = b_grid_rank;
    model->get_grid_size = b_grid_size;
    model->get_grid_type = b_grid_type;
    return model;
}
