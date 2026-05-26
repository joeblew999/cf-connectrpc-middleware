# connectrpc-cedar

## Intent

what we need is a connect rpc middleware to allow us to easily use cedar for authourisation.

## Connect rpc on cloudflare
The following wheel have already been invented to make this as easy as possible.

https://github.com/connyay/connectrpc-workers is a cloudflare workers implementation of connect rpc, allowing code gen for rpc, with clents in react.

https://github.com/connyay/example-multitenant-worker is an example thats multi tenant and so lends it self to cadar and importantly has a React GUI using connect rpc also.

https://github.com/connyay/example-connectrpc-worker is a set of simple examples.

## cedar 

https://github.com/cedar-policy/cedar has a browser wasm and so can also run on cloudflare too as wasm.


## tooling

we use mise in the root to mange all dependencies and nushell for scripting.

cedar cli might be useful too. 

https://github.com/cedar-policy/cedar-for-agents might be useful too. 

## GUI

React using Kumo. Love that Orange look !!

Easy to include https://kumo-ui.com/installation/

worth installing the cli : https://kumo-ui.com/cli ?

Colour referecnes: https://kumo-ui.com/colors/ so we can get the Orange look.

Maybe the Registry is useful ? https://kumo-ui.com/registry/












