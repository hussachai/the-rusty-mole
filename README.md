
Run to Me
===========

Run to Me is a dead simple local tunneling application written in Rust.

Server
===========
It is very easy to run the server. The most complicated part is setting up RabbitMQ, network configurations, and load balancer.
If you plan to have only one server, you can install RabbitMQ on the same machine, and use default values.

**Example**
The most common case, you install RabbitMQ in a separated instance (docker container or VM)
If you want the server to listen to a different port (default is 8080), you can specify `--port`.

```bash
./runtome-server --amqp-uri amqp://192.168.2.4:5672/%2f --port 9000
```
If you have RabbitMQ running on the same machine, you just simply run the binary.
```bash
./runtome-server
```

**Available Options**
- `bind-addr {ip}` The network address that this server will accept. The default value is `0.0.0.0` which means accept a request from any IP. 
- `amqp-uri {string}` The connection string for RabbitMQ. The default value is `amqp://127.0.0.1:5672/%2f`. Note that `%2f` is a URL encoded string for `/` which represents a virtual host `/`.
- `port {integer}` The port number that the server will listen to. The default value is 8080.


Client
===========

To run the client, you have to provide a target port that the client will route the traffic to. 
The client will try to reconnect with a simple backoff time. It will wait from 1 second before the next retry
and the wait time will be incremented by 1 second until it reaches 60 seconds. After that, the client will retry
every 60 seconds. If you have multiple instances of the server, this retry should be seamless since it will connect
to a new server automatically if the old one is down.

**Example**
```bash
./runtome --target-port 8888 --server-host https://tailrec.io --debug
```

Because `--server-host` parameter is something you don't change once the server is set up, this parameter can be provided/override 
through the environment variable named `RUNTOME_SERVER_HOST`. 
For Mac user, you can modify `~/.zprofile` and add the following line to that file.  
```text
export RUNTOME_SERVER_HOST=https://myownserver.xyz
```

**Available Options**
- `--target-port {integer}`* The HTTP port that the client will forward the request to.
- `--server-host {string}` The default value is `http://localhost:8080`.  
- `--debug` The flag indicating that all requests and responses will be printed to the screen.
- `--client-id {string}` The custom client ID. If it is not provided, UUID v4 without dashes will be used as a client ID.
- `--timeout-read {integer}` The socket read timeout in seconds. This is applied to the connection between the client and your server. The default value is 30 seconds.
- `--timeout-write {integer}` The socket write timeout in seconds. This is applied to the connection between the client and your server. The default value is 30 seconds.

Only `--target-port` argument is required. Other arguments are optional with sensible default values except `--server-host`
that you may want to change unless you want to run both server and client on the same machine (this makes sense for testing).

Development
===========

You can check out the code and test it out on your machine.

To run the server, you can use cargo run. All options are optional.  
`cargo run --bin runtome-server`

To run the client, you have to provide 
`cargo run --bin runtome -- --target-port 8888`

Note that `--` is used for passing command line arguments to the application.

