/* Minimal CSDMS BMI 2.0 C header (function-pointer struct).
 * Field order matches src/adapters/ffi.rs exactly — do not reorder. */
#ifndef BMI_H
#define BMI_H

#define BMI_SUCCESS (0)
#define BMI_FAILURE (1)

#define BMI_MAX_UNITS_NAME (2048)
#define BMI_MAX_TYPE_NAME (2048)
#define BMI_MAX_COMPONENT_NAME (2048)
#define BMI_MAX_VAR_NAME (2048)
#define BMI_MAX_LOCATION_NAME (2048)

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
    int (*get_value_at_indices)(struct Bmi *self, const char *name, void *dest,
                                int *inds, int count);

    int (*set_value)(struct Bmi *self, const char *name, void *src);
    int (*set_value_at_indices)(struct Bmi *self, const char *name, int *inds,
                                int count, void *src);

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

#endif
