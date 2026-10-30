/// The functions Koharu calls; bindgen brings the types they take.
const FUNCTIONS: &str = "^(sd_ctx_params_init|new_sd_ctx|free_sd_ctx|sd_ctx_supports_image_generation|sd_img_gen_params_init|generate_image|free_sd_image_generation_result|sd_set_log_callback|sd_commit)$";

fn main() {
    koharu_bindgen::generate(|bindings| {
        bindings
            .allowlist_function(FUNCTIONS)
            .prepend_enum_name(false)
    });
}
