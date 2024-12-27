# GraphDL Server

> "Entia non sunt multiplicanda praeter necessitatem."
> (Entities should not be multiplied beyond necessity.)
> -William of Ockham

A [`Thing`](https://schema.org/Thing) metamodel with admin interface.

The GraphDL (Graph Descriptor Language) framework describes and manipulates data within an Object-Role Model (ORM) graph structure. This framework provides a structured way to represent and interact with complex data relationships. The core components and concepts of GraphDL include:

**Nouns and Resources:** Represent entities, objects, or values within the system. Nouns can be any subject or direct object of a sentence. They serve as the primary nodes within a Graph Schema, embodying the entities around which relationships are formed. When a Graph Schema is implemented as a Graph, Nouns are stored as Resource nodes in the Graph. A Resource represents a single instance of a Noun, such as a user, product, order, etc.

**Verbs, Readings, and Roles:** Define the interactions between nouns. In a graph context, Readings with Roles can be thought of as the edges that connect nodes (Nouns), specifying the type of interaction or relationship that exists between them. This could include actions like "create", "modify", "owns", "relates to", etc. Readings provide the main interface for providing API definitions with verbs. Roles are used to define the directionality of the relationships between all nouns. When a Reading is accessed by an API, the Verb and Resources are stored as a Graph that implements that Reading's Graph Schema.

**Graph Schemas, Contraints, and Graphs:** Graph Schemas act as blueprints for how data is structured within a Graph. They define the readings of nouns and verbs that exist and how they are allowed to interact. Constraints ensure data consistency and govern the rules for how entities map to a relational database schema.

**Statuses and Transitions:** Statuses represent the status of an entity at a given time, and transitions define how entities move from one state to another based on certain verbs or events. This is crucial for modeling workflows and processes within a system.

**Events, Actions, and Guards:** These components detail the dynamics within the graph. Events trigger transitions between statuses, Actions are instances of Verbs that are used in a Graph, and Guards define rules or conditions that must be met for certain actions to take place or for transitions between states to occur.

GraphDL provides a comprehensive framework for modeling relationships and processes within a system, making it particularly suitable for applications that deal with complex data structures, such as e-commerce platforms, content management systems, and more. This framework allows developers, business analysts, and AIs to represent, query, and manipulate data in a way that mirrors real-world interactions and relationships.

This repo was created by running `npx create-payload-app@latest` and selecting the "blank" template.

![Relational Model](design/Schema.png)

![Graph](design/Core.png)

Open ORM model using Visual Studio 2022 with [NORMA](https://marketplace.visualstudio.com/items?itemName=ORMSolutions.NORMA2022) or explore online using <https://ormsolutions.com/tools/orm.aspx>

## Development

To spin up the project locally, follow these steps:

1. First, clone the repo
2. Then `cd YOUR_PROJECT_REPO && cp .env.example .env`
3. Next `yarn && yarn dev` (or `docker-compose up`, see [Docker](#docker))
4. Now `open http://localhost:8000/admin` to access the admin panel
5. Create your first admin user using the form on the page

Changes made in `./src` will be reflected in your app.

### Docker

Alternatively, you can use [Docker](https://www.docker.com) to spin up this project locally. To do so, follow these steps:

1. Follow [steps 1 and 2 from above](#development), the docker-compose file will automatically use the `.env` file in your project root
1. Next run `docker-compose up`
1. Follow [steps 4 and 5 from above](#development) to login and create your first admin user

The Docker instance will help you get up and running quickly while also standardizing the development environment across your teams.

### Deployment

The easiest way to deploy your project is to use [Payload Cloud](https://payloadcms.com/new/import), a one-click hosting solution to deploy production-ready instances of your Payload apps directly from your GitHub repo. You can also deploy your app manually, check out the [deployment documentation](https://payloadcms.com/docs/production/deployment) for full details.
