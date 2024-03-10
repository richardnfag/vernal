import yaml


with open("docker-compose.yml", "r") as file:
    compose_file = yaml.safe_load(file)

cpu = sum(
    [
        float(service[1].get("deploy").get("resources").get("limits").get("cpus"))
        for service in compose_file.get("services").items()
    ]
)


memory = sum(
    [
        float(
            service[1]
            .get("deploy")
            .get("resources")
            .get("limits")
            .get("memory")
            .replace("MB", "")
        )
        for service in compose_file.get("services").items()
    ]
)

print(f"Current: {cpu} CPU and {memory}MB of memory")

print("Maximum: 1.5 CPU and 550.0MB of memory")
