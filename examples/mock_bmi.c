/*
 * Simple Mock BMI Model for Testing
 * 
 * This is a minimal BMI implementation that you can compile and use
 * to test the Rust adapter. It simulates a simple diffusion model.
 *
 * Compile with:
 *   Linux:  gcc -shared -fPIC -o libmock_bmi.so mock_bmi.c
 *   macOS:  gcc -shared -fPIC -o libmock_bmi.dylib mock_bmi.c
 */

#include <stdlib.h>
#include <string.h>
#include <stdio.h>

/* BMI constants */
#define BMI_SUCCESS (0)
#define BMI_FAILURE (1)
#define BMI_MAX_COMPONENT_NAME (2048)
#define BMI_MAX_VAR_NAME (2048)
#define BMI_MAX_UNITS_NAME (2048)
#define BMI_MAX_TYPE_NAME (2048)

/* Forward declaration of Bmi struct */
struct Bmi;

/* Model state */
typedef struct {
    int initialized;
    double current_time;
    double start_time;
    double end_time;
    double time_step;
    int grid_size;
    double* temperature;  /* Main variable */
    double* heat_flux;    /* Input variable */
} MockModelState;

/* BMI struct definition (must match bmi.h) */
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
    int (*get_time_step)(struct Bmi *self, double *time_step);
    int (*get_value)(struct Bmi *self, const char *name, void *dest);
    int (*get_value_ptr)(struct Bmi *self, const char *name, void **dest_ptr);
    int (*get_value_at_indices)(struct Bmi *self, const char *name, void *dest, int *inds, int count);
    int (*set_value)(struct Bmi *self, const char *name, void *src);
    int (*set_value_at_indices)(struct Bmi *self, const char *name, int *inds, int count, void *src);
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
    int (*get_grid_edge_nodes)(struct Bmi *self, int grid, int *edge_nodes);
    int (*get_grid_face_edges)(struct Bmi *self, int grid, int *face_edges);
    int (*get_grid_face_nodes)(struct Bmi *self, int grid, int *face_nodes);
    int (*get_grid_nodes_per_face)(struct Bmi *self, int grid, int *nodes_per_face);
} Bmi;

/* Helper to get model state */
static MockModelState* get_state(Bmi* self) {
    return (MockModelState*)self->data;
}

/* BMI Function Implementations */

static int mock_initialize(Bmi *self, const char *config_file) {
    MockModelState* state = malloc(sizeof(MockModelState));
    if (!state) return BMI_FAILURE;
    
    /* Default configuration */
    state->initialized = 1;
    state->start_time = 0.0;
    state->end_time = 100.0;
    state->time_step = 1.0;
    state->current_time = 0.0;
    state->grid_size = 10;
    
    /* Simple config file parsing (just reads grid_size from first line) */
    if (config_file && strlen(config_file) > 0) {
        FILE* f = fopen(config_file, "r");
        if (f) {
            char line[256];
            while (fgets(line, sizeof(line), f)) {
                if (strncmp(line, "grid_size:", 10) == 0) {
                    state->grid_size = atoi(line + 10);
                } else if (strncmp(line, "end_time:", 9) == 0) {
                    state->end_time = atof(line + 9);
                } else if (strncmp(line, "time_step:", 10) == 0) {
                    state->time_step = atof(line + 10);
                }
            }
            fclose(f);
        }
    }
    
    /* Allocate arrays */
    state->temperature = calloc(state->grid_size, sizeof(double));
    state->heat_flux = calloc(state->grid_size, sizeof(double));
    
    if (!state->temperature || !state->heat_flux) {
        free(state->temperature);
        free(state->heat_flux);
        free(state);
        return BMI_FAILURE;
    }
    
    /* Initialize with some values */
    for (int i = 0; i < state->grid_size; i++) {
        state->temperature[i] = 20.0;  /* Initial temperature */
        state->heat_flux[i] = 0.0;
    }
    
    self->data = state;
    return BMI_SUCCESS;
}

static int mock_update(Bmi *self) {
    MockModelState* state = get_state(self);
    if (!state || !state->initialized) return BMI_FAILURE;
    
    /* Simple diffusion: T(t+1) = T(t) + heat_flux * dt */
    for (int i = 0; i < state->grid_size; i++) {
        state->temperature[i] += state->heat_flux[i] * state->time_step;
    }
    
    /* Simple internal diffusion between neighbors */
    double diffusion_coeff = 0.1;
    double* new_temp = malloc(state->grid_size * sizeof(double));
    for (int i = 0; i < state->grid_size; i++) {
        double left = (i > 0) ? state->temperature[i-1] : state->temperature[i];
        double right = (i < state->grid_size-1) ? state->temperature[i+1] : state->temperature[i];
        new_temp[i] = state->temperature[i] + 
                      diffusion_coeff * (left + right - 2*state->temperature[i]);
    }
    memcpy(state->temperature, new_temp, state->grid_size * sizeof(double));
    free(new_temp);
    
    state->current_time += state->time_step;
    return BMI_SUCCESS;
}

static int mock_update_until(Bmi *self, double then) {
    MockModelState* state = get_state(self);
    while (state->current_time < then) {
        if (mock_update(self) != BMI_SUCCESS) return BMI_FAILURE;
    }
    return BMI_SUCCESS;
}

static int mock_finalize(Bmi *self) {
    MockModelState* state = get_state(self);
    if (state) {
        free(state->temperature);
        free(state->heat_flux);
        free(state);
        self->data = NULL;
    }
    return BMI_SUCCESS;
}

static int mock_get_component_name(Bmi *self, char *name) {
    strcpy(name, "Mock Diffusion Model");
    return BMI_SUCCESS;
}

static int mock_get_input_item_count(Bmi *self, int *count) {
    *count = 1;  /* heat_flux */
    return BMI_SUCCESS;
}

static int mock_get_output_item_count(Bmi *self, int *count) {
    *count = 1;  /* temperature */
    return BMI_SUCCESS;
}

static int mock_get_input_var_names(Bmi *self, char **names) {
    strcpy(names[0], "heat_flux");
    return BMI_SUCCESS;
}

static int mock_get_output_var_names(Bmi *self, char **names) {
    strcpy(names[0], "temperature");
    return BMI_SUCCESS;
}

static int mock_get_var_grid(Bmi *self, const char *name, int *grid) {
    *grid = 0;  /* Single grid */
    return BMI_SUCCESS;
}

static int mock_get_var_type(Bmi *self, const char *name, char *type) {
    strcpy(type, "double");
    return BMI_SUCCESS;
}

static int mock_get_var_units(Bmi *self, const char *name, char *units) {
    if (strcmp(name, "temperature") == 0) {
        strcpy(units, "K");
    } else if (strcmp(name, "heat_flux") == 0) {
        strcpy(units, "W/m^2");
    } else {
        strcpy(units, "-");
    }
    return BMI_SUCCESS;
}

static int mock_get_var_itemsize(Bmi *self, const char *name, int *size) {
    *size = sizeof(double);
    return BMI_SUCCESS;
}

static int mock_get_var_nbytes(Bmi *self, const char *name, int *nbytes) {
    MockModelState* state = get_state(self);
    *nbytes = state->grid_size * sizeof(double);
    return BMI_SUCCESS;
}

static int mock_get_var_location(Bmi *self, const char *name, char *location) {
    strcpy(location, "node");
    return BMI_SUCCESS;
}

static int mock_get_current_time(Bmi *self, double *time) {
    *time = get_state(self)->current_time;
    return BMI_SUCCESS;
}

static int mock_get_start_time(Bmi *self, double *time) {
    *time = get_state(self)->start_time;
    return BMI_SUCCESS;
}

static int mock_get_end_time(Bmi *self, double *time) {
    *time = get_state(self)->end_time;
    return BMI_SUCCESS;
}

static int mock_get_time_units(Bmi *self, char *units) {
    strcpy(units, "s");
    return BMI_SUCCESS;
}

static int mock_get_time_step(Bmi *self, double *time_step) {
    *time_step = get_state(self)->time_step;
    return BMI_SUCCESS;
}

static int mock_get_value(Bmi *self, const char *name, void *dest) {
    MockModelState* state = get_state(self);
    if (strcmp(name, "temperature") == 0) {
        memcpy(dest, state->temperature, state->grid_size * sizeof(double));
    } else if (strcmp(name, "heat_flux") == 0) {
        memcpy(dest, state->heat_flux, state->grid_size * sizeof(double));
    } else {
        return BMI_FAILURE;
    }
    return BMI_SUCCESS;
}

static int mock_get_value_ptr(Bmi *self, const char *name, void **dest_ptr) {
    MockModelState* state = get_state(self);
    if (strcmp(name, "temperature") == 0) {
        *dest_ptr = state->temperature;
    } else if (strcmp(name, "heat_flux") == 0) {
        *dest_ptr = state->heat_flux;
    } else {
        return BMI_FAILURE;
    }
    return BMI_SUCCESS;
}

static int mock_get_value_at_indices(Bmi *self, const char *name, void *dest, int *inds, int count) {
    MockModelState* state = get_state(self);
    double* src = NULL;
    
    if (strcmp(name, "temperature") == 0) {
        src = state->temperature;
    } else if (strcmp(name, "heat_flux") == 0) {
        src = state->heat_flux;
    } else {
        return BMI_FAILURE;
    }
    
    double* d = (double*)dest;
    for (int i = 0; i < count; i++) {
        d[i] = src[inds[i]];
    }
    return BMI_SUCCESS;
}

static int mock_set_value(Bmi *self, const char *name, void *src) {
    MockModelState* state = get_state(self);
    if (strcmp(name, "heat_flux") == 0) {
        memcpy(state->heat_flux, src, state->grid_size * sizeof(double));
    } else if (strcmp(name, "temperature") == 0) {
        memcpy(state->temperature, src, state->grid_size * sizeof(double));
    } else {
        return BMI_FAILURE;
    }
    return BMI_SUCCESS;
}

static int mock_set_value_at_indices(Bmi *self, const char *name, int *inds, int count, void *src) {
    MockModelState* state = get_state(self);
    double* dest = NULL;
    
    if (strcmp(name, "heat_flux") == 0) {
        dest = state->heat_flux;
    } else if (strcmp(name, "temperature") == 0) {
        dest = state->temperature;
    } else {
        return BMI_FAILURE;
    }
    
    double* s = (double*)src;
    for (int i = 0; i < count; i++) {
        dest[inds[i]] = s[i];
    }
    return BMI_SUCCESS;
}

static int mock_get_grid_rank(Bmi *self, int grid, int *rank) {
    *rank = 1;  /* 1D grid */
    return BMI_SUCCESS;
}

static int mock_get_grid_size(Bmi *self, int grid, int *size) {
    *size = get_state(self)->grid_size;
    return BMI_SUCCESS;
}

static int mock_get_grid_type(Bmi *self, int grid, char *type) {
    strcpy(type, "uniform_rectilinear");
    return BMI_SUCCESS;
}

static int mock_get_grid_shape(Bmi *self, int grid, int *shape) {
    shape[0] = get_state(self)->grid_size;
    return BMI_SUCCESS;
}

static int mock_get_grid_spacing(Bmi *self, int grid, double *spacing) {
    spacing[0] = 1.0;  /* Unit spacing */
    return BMI_SUCCESS;
}

static int mock_get_grid_origin(Bmi *self, int grid, double *origin) {
    origin[0] = 0.0;
    return BMI_SUCCESS;
}

static int mock_get_grid_x(Bmi *self, int grid, double *x) {
    MockModelState* state = get_state(self);
    for (int i = 0; i < state->grid_size; i++) {
        x[i] = (double)i;
    }
    return BMI_SUCCESS;
}

static int mock_get_grid_y(Bmi *self, int grid, double *y) {
    return BMI_FAILURE;  /* 1D grid, no Y */
}

static int mock_get_grid_z(Bmi *self, int grid, double *z) {
    return BMI_FAILURE;  /* 1D grid, no Z */
}

static int mock_get_grid_node_count(Bmi *self, int grid, int *count) {
    *count = get_state(self)->grid_size;
    return BMI_SUCCESS;
}

static int mock_get_grid_edge_count(Bmi *self, int grid, int *count) {
    *count = get_state(self)->grid_size - 1;
    return BMI_SUCCESS;
}

static int mock_get_grid_face_count(Bmi *self, int grid, int *count) {
    *count = 0;
    return BMI_SUCCESS;
}

static int mock_get_grid_edge_nodes(Bmi *self, int grid, int *edge_nodes) {
    MockModelState* state = get_state(self);
    for (int i = 0; i < state->grid_size - 1; i++) {
        edge_nodes[i*2] = i;
        edge_nodes[i*2 + 1] = i + 1;
    }
    return BMI_SUCCESS;
}

static int mock_get_grid_face_edges(Bmi *self, int grid, int *face_edges) {
    return BMI_SUCCESS;  /* No faces in 1D */
}

static int mock_get_grid_face_nodes(Bmi *self, int grid, int *face_nodes) {
    return BMI_SUCCESS;  /* No faces in 1D */
}

static int mock_get_grid_nodes_per_face(Bmi *self, int grid, int *nodes_per_face) {
    return BMI_SUCCESS;  /* No faces in 1D */
}

/* 
 * Registration function - this is what the Rust adapter calls!
 * The name of this function should match what you pass to BmiModel::load()
 */
Bmi* register_bmi_mock(Bmi* model) {
    if (!model) return NULL;
    
    model->data = NULL;
    
    /* Set all function pointers */
    model->initialize = mock_initialize;
    model->update = mock_update;
    model->update_until = mock_update_until;
    model->finalize = mock_finalize;
    
    model->get_component_name = mock_get_component_name;
    model->get_input_item_count = mock_get_input_item_count;
    model->get_output_item_count = mock_get_output_item_count;
    model->get_input_var_names = mock_get_input_var_names;
    model->get_output_var_names = mock_get_output_var_names;
    
    model->get_var_grid = mock_get_var_grid;
    model->get_var_type = mock_get_var_type;
    model->get_var_units = mock_get_var_units;
    model->get_var_itemsize = mock_get_var_itemsize;
    model->get_var_nbytes = mock_get_var_nbytes;
    model->get_var_location = mock_get_var_location;
    
    model->get_current_time = mock_get_current_time;
    model->get_start_time = mock_get_start_time;
    model->get_end_time = mock_get_end_time;
    model->get_time_units = mock_get_time_units;
    model->get_time_step = mock_get_time_step;
    
    model->get_value = mock_get_value;
    model->get_value_ptr = mock_get_value_ptr;
    model->get_value_at_indices = mock_get_value_at_indices;
    model->set_value = mock_set_value;
    model->set_value_at_indices = mock_set_value_at_indices;
    
    model->get_grid_rank = mock_get_grid_rank;
    model->get_grid_size = mock_get_grid_size;
    model->get_grid_type = mock_get_grid_type;
    model->get_grid_shape = mock_get_grid_shape;
    model->get_grid_spacing = mock_get_grid_spacing;
    model->get_grid_origin = mock_get_grid_origin;
    
    model->get_grid_x = mock_get_grid_x;
    model->get_grid_y = mock_get_grid_y;
    model->get_grid_z = mock_get_grid_z;
    
    model->get_grid_node_count = mock_get_grid_node_count;
    model->get_grid_edge_count = mock_get_grid_edge_count;
    model->get_grid_face_count = mock_get_grid_face_count;
    model->get_grid_edge_nodes = mock_get_grid_edge_nodes;
    model->get_grid_face_edges = mock_get_grid_face_edges;
    model->get_grid_face_nodes = mock_get_grid_face_nodes;
    model->get_grid_nodes_per_face = mock_get_grid_nodes_per_face;
    
    return model;
}
