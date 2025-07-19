
# bento.rs


## What is Bento?
A rootless, daemonless, low level container runtime for Linux written in Rust that aims to be compliant with the OCI Runtime Spec.

## Testing

To test the container creation, you need to set up a test bundle.

1.  Create the bundle directory structure:
    ```
    mkdir -p test-bundle/rootfs/bin
    ```

2.  Copy a shell into the rootfs. On most Linux systems, you can use:
    ```
    cp /bin/sh test-bundle/rootfs/bin/
    ```

3.  Create a `config.json` file inside the `test-bundle` directory with the desired container configuration.

