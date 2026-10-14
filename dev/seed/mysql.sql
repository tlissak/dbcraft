-- A tiny original sample schema (films and rentals) for driver tests and demos.
create table actor (actor_id int auto_increment primary key, first_name varchar(45) not null, last_name varchar(45) not null);
create table film (
  film_id int unsigned auto_increment primary key,
  title varchar(128) not null,
  release_year year,
  rating decimal(3,1),
  added_at timestamp default current_timestamp,
  meta json
);
create table film_actor (actor_id int, film_id int unsigned, primary key (actor_id, film_id),
  foreign key (actor_id) references actor(actor_id), foreign key (film_id) references film(film_id));
create view film_list as select f.title, concat(a.first_name, ' ', a.last_name) as actor from film f join film_actor fa using (film_id) join actor a using (actor_id);

insert into actor (first_name, last_name) values ('Ada', 'Stone'), ('Ben', 'Okafor'), ('Chen', 'Li');
insert into film (title, release_year, rating, meta) values
  ('The Quiet Harbor', 2019, 7.4, '{"genre":"drama"}'),
  ('Orbit of Glass', 2021, 8.1, '{"genre":"sci-fi"}'),
  ('Paper Lanterns', 2016, 6.9, null);
insert into film_actor values (1,1),(2,1),(2,2),(3,3);
